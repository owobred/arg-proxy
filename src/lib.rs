#![feature(iter_array_chunks)]

use discord::{fetch_new_url, DiscordUrl, ExpiryParameters};
use prost::Message;
use worker::*;

mod discord;

const EXPIRY_BUFFER: time::Duration = time::Duration::minutes(30);

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
                return Response::redirect(parsed.to_string().parse().unwrap());
                // return Response::ok("url not yet expired (no update)");
            } else {
                add_to_kv(&parsed, &kv).await?;
                return Response::redirect(parsed.to_string().parse().unwrap());
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
                return Response::redirect(stored_url.to_string().parse().unwrap());
                // return Response::ok(format!("url stored is not expired yet: {}", stored_url.to_string()));
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

    return Response::redirect(new_parsed.to_string().parse().unwrap());
    // Response::ok(format!("fetched new url {}", new_parsed.to_string()))
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
    use crate::{discord::DiscordUrl, ExpiryParameters};

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
