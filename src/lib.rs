#![feature(iter_array_chunks)]

use std::collections::HashMap;

use worker::*;

const DISCORD_USERAGENT: &'static str = "DiscordBot (github.com/owobred/arg-proxy; v0.0.1)";
const DSICORD_REFRESH_ROUTE: &'static str = "https://discord.com/api/v9/attachments/refresh-urls";

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

    Response::ok(format!("{parsed:?}"))
}

#[derive(Debug)]
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
    fn try_from_full_url(url: &Url) -> std::result::Result<DiscordUrl, InnerError> {
        let Ok(inner_url) = Url::parse(&url.path()[1..]) else {
            return Err(InnerError::ParseError("failed to parse inner url"));
        };

        let mut path = inner_url
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

    fn to_kv_key(&self) -> String {
        format!("{:x}/{:x}", self.channel_id, self.attachment_id)
    }
}

#[derive(Debug)]
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
