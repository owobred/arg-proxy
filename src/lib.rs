#![feature(iter_array_chunks)]

use std::collections::HashMap;

use prost::Message;
use serde::{Deserialize, Serialize};
use worker::*;

const DISCORD_USERAGENT: &'static str = "DiscordBot (github.com/owobred/arg-proxy; v0.0.1)";
const DISCORD_REFRESH_ROUTE: &'static str = "https://discord.com/api/v9/attachments/refresh-urls";

const EXPIRY_BUFFER: time::Duration = time::Duration::minutes(30);
const HEADER_AUTHORIZATION: &'static str = "Authorization";
const HEADER_CONTENT_TYPE: &'static str = "Content-Type";
const HEADER_ACCEPT: &'static str = "Accept";
const HEADER_REQUEST_INFO: &'static str = "X-Request-Info";

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let Ok(kv) = env.kv("arg_cdn") else {
        return Response::error("could not find kv store", 500);
    };

    let Ok(url) = req.url() else {
        return Response::error("failed to parse url??", 500);
    };

    let now = time::OffsetDateTime::now_utc();

    let parsed = match DiscordUrl::try_from_full_url(&url) {
        Ok(parsed) => parsed,
        Err(error) => match error {
            InnerError::ParseError(message) => {
                return Response::error(format!("parse error: {message}"), 400)
            }
            other => {
                console_error!("unhandled error parsing discord url: {other:?}");
                return Response::error("unexpected error", 500);
            }
        },
    };

    if let Some(expiry) = &parsed.expiry_params {
        if expiry.expiry - EXPIRY_BUFFER > now {
            let existing: Option<DiscordUrl> = kv
                .get(&parsed.to_kv_key())
                .bytes()
                .await?
                .map(|bytes| proto::Stored::decode(std::io::Cursor::new(bytes)).unwrap())
                .map(|stored| stored.try_into().unwrap());

            let same = existing == Some(parsed.clone());

            if same {
                return Response::redirect(parsed.to_string().parse().unwrap()).map(|mut r| {
                    let headers = r.headers_mut();
                    headers
                        .append(HEADER_REQUEST_INFO, "already_stored")
                        .unwrap();
                    r
                });
                // return Response::ok("url not yet expired (no update)");
            } else {
                add_to_kv(&parsed, &kv).await?;
                return Response::redirect(parsed.to_string().parse().unwrap()).map(|mut r| {
                    let headers = r.headers_mut();
                    headers
                        .append(HEADER_REQUEST_INFO, "new_not_expired")
                        .unwrap();
                    r
                });
                // return Response::ok("url not yet expired (update)");
            }
        }
    }

    let from_kv = kv.get(&parsed.to_kv_key()).bytes().await?;

    if let Some(bytes) = from_kv {
        let stored = match proto::Stored::decode(std::io::Cursor::new(bytes)) {
            Ok(stored) => stored,
            Err(error) => {
                console_error!("failed to decode value from kv {error:?}");

                return Response::error("internal server error", 500);
            }
        };

        let stored_url: DiscordUrl = match stored.try_into() {
            Ok(url) => url,
            Err(error) => {
                console_error!("failed to convert from stored to url: {error:?}");
                return Response::error("internal server error", 500);
            }
        };

        if let Some(expiry) = &stored_url.expiry_params {
            if expiry.expiry - EXPIRY_BUFFER > now {
                return Response::redirect(stored_url.to_string().parse().unwrap()).map(|mut r| {
                    let headers = r.headers_mut();
                    headers.append(HEADER_REQUEST_INFO, "stored_not_expired").unwrap();
                    r
                });
            }
        }
    }

    let discord_token = env.secret("DISCORD_TOKEN")?.to_string();
    let new_url = match fetch_new_url(&parsed, &discord_token).await {
        Ok(urls) => urls.refreshed_urls.into_iter().next().unwrap().refreshed,
        Err(error) => {
            console_error!("failed to fetch new url: {error}");
            return Response::error("internal server error", 500);
        }
    };
    let new_parsed = match DiscordUrl::try_from_url(&Url::parse(&new_url)?) {
        Ok(parsed) => parsed,
        Err(error) => {
            console_error!("failed to parse new discord attachment url: {error:?}");
            return Response::error("internal server error", 500);
        }
    };
    add_to_kv(&new_parsed, &kv).await?;

    return Response::redirect(new_parsed.to_string().parse().unwrap()).map(|mut r| {
        let headers = r.headers_mut();
        headers.append(HEADER_REQUEST_INFO, "expired").unwrap();
        r
    });
}

async fn add_to_kv(url: &DiscordUrl, kv: &kv::KvStore) -> Result<()> {
    let stored: proto::Stored = url.to_owned().try_into().unwrap();
    let mut stored_buf = Vec::new();
    stored.encode(&mut stored_buf).unwrap();

    kv.put_bytes(&url.to_kv_key(), &stored_buf)?
        .execute()
        .await?;
    Ok(())
}

async fn fetch_new_url(
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
struct DiscordRenewAttachmentResponse {
    refreshed_urls: Vec<DiscordRefreshedUrl>,
}

#[derive(Debug, Deserialize)]
struct DiscordRefreshedUrl {
    // original: String,
    refreshed: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiscordUrl {
    channel_id: u64,
    attachment_id: u64,
    filename: String,
    expiry_params: Option<ExpiryParameters>,
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
    fn try_from_url(url: &Url) -> std::result::Result<DiscordUrl, InnerError> {
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

    fn try_from_full_url(url: &Url) -> std::result::Result<DiscordUrl, InnerError> {
        let Ok(mut inner_url) = Url::parse(&url.path()[1..]) else {
            return Err(InnerError::ParseError("failed to parse inner url"));
        };

        inner_url.set_query(url.query());

        Self::try_from_url(&inner_url)
    }

    fn to_kv_key(&self) -> String {
        format!("{:x}/{:x}", self.channel_id, self.attachment_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExpiryParameters {
    expiry: time::OffsetDateTime,
    is: time::OffsetDateTime,
    hm: Vec<u8>,
}

impl ExpiryParameters {
    fn try_from_params_map(
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

struct StoredAttachment {
    filename: String,
    channel_id: u64,
    attachment_id: u64,
    latest_expiry_parameters: ExpiryParameters,
}

#[derive(Debug, thiserror::Error)]
enum InnerError {
    #[error("failed to parse")]
    ParseError(&'static str),
    #[error("problem with request")]
    RequestError(#[from] reqwest::Error),
    #[error("problem with serde_json")]
    SerdeJsonError(#[from] serde_json::Error),
    #[error("something unexpected happened")]
    Other,
}
mod proto {
    use crate::{DiscordUrl, ExpiryParameters};

    include!(concat!(env!("OUT_DIR"), "/arg_proxy.rs"));

    impl From<DiscordUrl> for Stored {
        fn from(value: DiscordUrl) -> Self {
            Self {
                file_name: value.filename,
                channel_id: value.channel_id,
                attachment_id: value.attachment_id,
                expiry_info: value.expiry_params.map(|p| p.into()),
            }
        }
    }

    impl From<ExpiryParameters> for Expiry {
        fn from(value: ExpiryParameters) -> Self {
            Self {
                expiry_time_seconds: value.expiry.unix_timestamp(),
                is_seconds: value.is.unix_timestamp(),
                hm: value.hm,
            }
        }
    }
}

impl TryFrom<proto::Stored> for DiscordUrl {
    type Error = time::Error;

    fn try_from(value: proto::Stored) -> std::result::Result<Self, Self::Error> {
        let expiry_params = match value.expiry_info {
            Some(params) => Some(params.try_into()?),
            None => None,
        };

        Ok(Self {
            channel_id: value.channel_id,
            attachment_id: value.attachment_id,
            filename: value.file_name,
            expiry_params,
        })
    }
}

impl TryFrom<proto::Expiry> for ExpiryParameters {
    type Error = time::Error;

    fn try_from(value: proto::Expiry) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            expiry: time::OffsetDateTime::from_unix_timestamp(value.expiry_time_seconds)?,
            is: time::OffsetDateTime::from_unix_timestamp(value.is_seconds)?,
            hm: value.hm,
        })
    }
}
