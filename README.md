## How to use
1. [install wrangler](https://developers.cloudflare.com/workers/wrangler/install-and-update/)
2. create a new cloudflare KV namespace named `arg_cdn`
3. place KV info into `wrangler.toml`
4. place your cloudflare account id into `wrangler.toml`
5. [create a discord bot](https://discord.com/developers/applications)
6. add your discord bot token using `npx wrangler secret put DISCORD_TOKEN`, and then enter the token when prompted
7. run `npx wrangler deploy`

### Development
1. ensure you have the wasm32-unknown-unknown target installed for rust
2. add discord bot token to `.dev.vars`
3. run `npx wrangler dev`