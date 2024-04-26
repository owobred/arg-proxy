use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::InnerError;

const DISCORD_USERAGENT: &'static str = "DiscordBot (github.com/owobred/arg-proxy; v0.0.1)";
const DISCORD_REFRESH_ROUTE: &'static str = "https://discord.com/api/v9/attachments/refresh-urls";

const HEADER_AUTHORIZATION: &'static str = "Authorization";
const HEADER_CONTENT_TYPE: &'static str = "Content-Type";
const HEADER_ACCEPT: &'static str = "Accept";

pub async fn fetch_new_url(
    url: &DiscordUrl,
    token: &str,
) -> std::result::Result<DiscordRenewAttachmentResponse, InnerError> {
    let request_body = DiscordRenewAttachmentRequest {
        attachment_urls: vec![url.to_string()],
    };

    let client = reqwest::Client::builder()
        .user_agent(DISCORD_USERAGENT)
        .build()?;
    let response: DiscordRenewAttachmentResponse = client
        .post(DISCORD_REFRESH_ROUTE)
        .body(serde_json::to_string(&request_body)?)
        .header(HEADER_AUTHORIZATION, format!("Bot {token}"))
        .header(HEADER_CONTENT_TYPE, "application/json")
        .header(HEADER_ACCEPT, "application/json")
        .send()
        .await?
        .json()
        .await?;

    Ok(response)
}

#[derive(Debug, Serialize)]
struct DiscordRenewAttachmentRequest {
    attachment_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct DiscordRenewAttachmentResponse {
    pub refreshed_urls: Vec<DiscordRefreshedUrl>,
}

#[derive(Debug, Deserialize)]
pub struct DiscordRefreshedUrl {
    pub refreshed: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordUrl {
    pub channel_id: u64,
    pub attachment_id: u64,
    pub filename: String,
    pub expiry_params: Option<ExpiryParameters>,
}

impl ToString for DiscordUrl {
    fn to_string(&self) -> String {
        if let Some(expiry) = &self.expiry_params {
            format!(
                "https://cdn.discordapp.com/attachments/{}/{}/{}?ex={:x}&is={:x}&hm={}",
                self.channel_id,
                self.attachment_id,
                self.filename,
                expiry.expiry.unix_timestamp(),
                expiry.is.unix_timestamp(),
                expiry
                    .hm
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            )
        } else {
            format!(
                "https://cdn.discordapp.com/attachments/{}/{}/{}",
                self.channel_id, self.attachment_id, self.filename
            )
        }
    }
}

impl DiscordUrl {
    pub fn try_from_url(url: &Url) -> std::result::Result<DiscordUrl, InnerError> {
        let mut path = url
            .path_segments()
            .ok_or(InnerError::ParseError("missing attachment section"))?
            .skip(1);
        let channel_id: u64 = path
            .next()
            .ok_or(InnerError::ParseError("missing channel_id section"))?
            .parse()
            .map_err(|_| InnerError::ParseError("failed to parse channel_id section"))?;
        let attachment_id: u64 = path
            .next()
            .ok_or(InnerError::ParseError("missing attachment_id section"))?
            .parse()
            .map_err(|_| InnerError::ParseError("failed to parse attachment_id section"))?;
        let filename = path
            .next()
            .ok_or(InnerError::ParseError("missing filename section"))?;

        let expiry_params = match ExpiryParameters::try_from_params_map(
            url.query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        ) {
            Some(params) => Some(params?),
            None => None,
        };

        Ok(DiscordUrl {
            channel_id,
            attachment_id,
            filename: filename.to_string(),
            expiry_params,
        })
    }

    pub fn try_from_full_url(url: &Url) -> std::result::Result<DiscordUrl, InnerError> {
        let Ok(mut inner_url) = Url::parse(&url.path()[1..]) else {
            return Err(InnerError::ParseError("failed to parse inner url"));
        };

        inner_url.set_query(url.query());

        Self::try_from_url(&inner_url)
    }

    pub fn to_kv_key(&self) -> String {
        format!("{:x}/{:x}", self.channel_id, self.attachment_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiryParameters {
    pub expiry: time::OffsetDateTime,
    pub is: time::OffsetDateTime,
    pub hm: Vec<u8>,
}

impl ExpiryParameters {
    pub fn try_from_params_map(
        params: HashMap<String, String>,
    ) -> Option<std::result::Result<Self, InnerError>> {
        let expiry = match i64::from_str_radix(&params.get("ex")?, 16)
            .map_err(|_| InnerError::ParseError("failed to parse ex parameter"))
            .and_then(|val| {
                time::OffsetDateTime::from_unix_timestamp(val)
                    .map_err(|_| InnerError::ParseError("failed to parse ex timestamp"))
            }) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        let is = match i64::from_str_radix(&params.get("is")?, 16)
            .map_err(|_| InnerError::ParseError("failed to parse is parameter"))
            .and_then(|val| {
                time::OffsetDateTime::from_unix_timestamp(val)
                    .map_err(|_| InnerError::ParseError("failed to parse is timestamp"))
            }) {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        let hm = match params
            .get("hm")?
            .to_string()
            .chars()
            .array_chunks::<2>()
            .map(|v| u8::from_str_radix(&v.into_iter().collect::<String>(), 16))
            .map(|v| v.map_err(|_| InnerError::ParseError("failed to parse hm byte")))
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(hm) => hm,
            Err(err) => return Some(Err(err)),
        };

        Some(Ok(ExpiryParameters { expiry, is, hm }))
    }
}
