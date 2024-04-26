#![feature(iter_array_chunks)]

// use tracing::info;
use worker::*;

#[event(fetch)]
async fn main(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    // let router = Router::new();
    // let Ok(kv) = env.kv("arg") else { return Response::error("could not find kv store", 500) };

    let Ok(url) = req.url() else {
        return Response::error("failed to parse url??", 500);
    };
    console_log!("parsing discord url");
    let Some(parsed) = parse_url(&url) else {
        return Response::error("invalid url passed", 400);
    };

    console_log!("got parsed url {parsed:?}");

    let now = time::OffsetDateTime::now_utc();
    let now_unix_seconds = now.unix_timestamp();
    console_log!("it is currently {now_unix_seconds}");

    let expiry_parameter = url
        .query_pairs()
        .inspect(|kv| console_log!("had kv {kv:?}"))
        .find(|(k, _)| k == "ex");

    if let Some((_, expires_at)) = expiry_parameter {
        let Ok(expires_at_seconds) = u64::from_str_radix(&expires_at, 16) else {
            return Response::error("invalid expiry in url", 400);
        };

        let expired = expires_at_seconds < now_unix_seconds as u64;

        if !expired {
            // TODO: insert current into kv
            return Response::ok(format!("redirect to {}", parsed.to_string()));
        }
    }

    // console_log!("got ");

    Response::ok(format!("{parsed:?}"))

    // router.get_async("/*path", |mut req, ctx| async move {
    //     console_log!("got req {:?}", req.path());
    //     ctx.v
    //     let path = &ctx.param("path").unwrap()[1..];
    //     console_log!("got path {path}");
    //     let Ok(url) = Url::parse(path) else { return Response::error("invalid url", 400) };
    //     console_log!("got url {url:?}");

    //     let mut segments = url.path_segments().unwrap();
    //     segments.next();  // skip /attachments/
    //     console_log!("remaining {}", segments.clone().collect::<Vec<&str>>().join("@@"));
    //     let channel_id: u64 = segments.next().unwrap().parse().unwrap();
    //     let attachment_id: u64 = segments.next().unwrap().parse().unwrap();
    //     let filename = segments.next().unwrap();
    //     console_log!("split segments {channel_id} {attachment_id} {filename}");
    //     // info!(?channel_id, ?attachment_id, ?filename, "checking file");

    //     let (_, expires_at) = url.query_pairs().inspect(|kv| console_log!("had kv {kv:?}")).find(|(k, _)| k == "ex").unwrap();
    //     let expires_at_unix_seconds = u64::from_str_radix(&expires_at, 16).unwrap();

    //     // let kv = ctx.kv("arg_proxy").unwrap();
    //     // let path_info = kv.get(&format!("{channel_id}/{attachment_id}/{filename}"));

    //     Response::ok(format!("expires at {expires_at_unix_seconds}"))
    //     // Response::ok(url.host().unwrap().to_string())
    // }).run(req, env).await
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

fn parse_url(url: &Url) -> Option<DiscordUrl> {
    let Ok(inner_url) = Url::parse(&url.path()[1..]) else {
        return None;
    };

    let mut path = inner_url.path_segments()?.skip(1);
    let channel_id: u64 = path.next().unwrap().parse().unwrap();
    let attachment_id: u64 = path.next().unwrap().parse().unwrap();
    let filename = path.next().unwrap();

    console_log!("checking for params");
    let has_all_params =
        url.query_pairs().map(|(k, _)| k).collect::<Vec<_>>() == &["ex", "is", "hm"];

    let expiry_params = if has_all_params {
        console_log!("had expiry params");
        let expiry = url
            .query_pairs()
            .find_map(|(k, v)| {
                (k == "ex")
                    .then(|| time::OffsetDateTime::from_unix_timestamp(i64::from_str_radix(&v, 16).unwrap()).unwrap())
            })
            .unwrap();
        console_log!("had expiry {expiry:?}");
        let is = url
        .query_pairs()
        .find_map(|(k, v)| {
            (k == "is")
            .then(|| time::OffsetDateTime::from_unix_timestamp(i64::from_str_radix(&v, 16).unwrap()).unwrap())
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
                        .map(|v| u8::from_str_radix(&v.into_iter().collect::<String>(), 16).unwrap())
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

    Some(DiscordUrl {
        channel_id,
        attachment_id,
        filename: filename.to_string(),
        expiry_params,
    })
}