#![feature(iter_array_chunks)]

use worker::*;

const DISCORD_USERAGENT: &'static str = "DiscordBot (github.com/owobred/arg-proxy; v0.0.1)";

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // let Ok(kv) = env.kv("arg_cdn") else { return Response::error("could not find kv store", 500) };

    let Ok(url) = req.url() else {
        return Response::error("failed to parse url??", 500);
    };

    let now = time::OffsetDateTime::now_utc();

    let parsed = match parse_url(&url) {
        Ok(parsed) => parsed,
        Err(error) => {
            match error {
                InnerError::ParseError(message) => return Response::error(format!("parse error: {message}"), 400),
                other => {
                    console_error!("unhandled error parsing discord url: {other:?}");
                    return Response::error("unexpected error", 500);
                },
            }
        }
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

#[derive(Debug)]
struct ExpiryParameters {
    expiry: time::OffsetDateTime,
    is: time::OffsetDateTime,
    hm: Vec<u8>,
}

fn parse_url(url: &Url) -> std::result::Result<DiscordUrl, InnerError> {
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

    console_log!("checking for params");
    let has_all_params =
        url.query_pairs().map(|(k, _)| k).collect::<Vec<_>>() == &["ex", "is", "hm"];

    let expiry_params = if has_all_params {
        console_log!("had expiry params");
        let expiry = url
            .query_pairs()
            .find_map(|(k, v)| {
                (k == "ex").then(|| {
                    time::OffsetDateTime::from_unix_timestamp(i64::from_str_radix(&v, 16).unwrap())
                        .unwrap()
                })
            })
            .unwrap();
        console_log!("had expiry {expiry:?}");
        let is = url
            .query_pairs()
            .find_map(|(k, v)| {
                (k == "is").then(|| {
                    time::OffsetDateTime::from_unix_timestamp(i64::from_str_radix(&v, 16).unwrap())
                        .unwrap()
                })
            })
            .unwrap();
        console_log!("had is {is:?}");
        let hm = url
            .query_pairs()
            .find_map(|(k, v)| {
                (k == "hm").then(|| {
                    v.to_string()
                        .chars()
                        .array_chunks::<2>()
                        .map(|v| {
                            u8::from_str_radix(&v.into_iter().collect::<String>(), 16).unwrap()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap();
        console_log!("had hm {hm:x?}");

        Some(ExpiryParameters { expiry, is, hm })
    } else {
        console_log!("had no params");
        None
    };

    Ok(DiscordUrl {
        channel_id,
        attachment_id,
        filename: filename.to_string(),
        expiry_params,
    })
}

#[derive(Debug, thiserror::Error)]
enum InnerError {
    #[error("failed to parse")]
    ParseError(&'static str),
    #[error("something unexpected happened")]
    Other,
}
