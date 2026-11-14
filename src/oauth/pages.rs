//! The few HTML pages of the authorization flow. Every dynamic value passes
//! through `escape`: client names are chosen by whoever registers a client.

const STYLE: &str = "body{font:16px/1.5 system-ui,sans-serif;max-width:32rem;margin:4rem auto;\
padding:0 1rem;color:#1f2328}h1{font-size:1.4rem}.target{font-weight:600}\
.actions{display:flex;gap:.75rem;margin-top:1.5rem}\
button,a.button{font:inherit;padding:.5rem 1rem;border:1px solid #d1d9e0;border-radius:6px;\
background:#f6f8fa;color:inherit;text-decoration:none;cursor:pointer}\
button.primary,a.button.primary{background:#0866ff;border-color:#0866ff;color:#fff}";

fn layout(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title}</title><style>{STYLE}</style></head><body><h1>{title}</h1>{body}</body></html>",
        title = escape(title),
    )
}

pub fn consent(client_name: Option<&str>, target: &str, consent_token: &str) -> String {
    let name = client_name
        .map(|name| format!("<p>The client calls itself “{}”.</p>", escape(name)))
        .unwrap_or_default();
    layout(
        "Connect Instagram?",
        &format!(
            "<p><span class=\"target\">{target}</span> wants to read your Instagram \
             professional accounts: profile, posts, comments, insights, and hashtag and \
             competitor data. It cannot post or change anything.</p>{name}\
             <p>Only continue if you started this yourself. You will sign in with Facebook \
             next and choose which Pages and Instagram accounts to share.</p>\
             <form method=\"post\" action=\"/authorize/consent\">\
             <input type=\"hidden\" name=\"consent\" value=\"{token}\">\
             <div class=\"actions\">\
             <button class=\"primary\" name=\"decision\" value=\"allow\">Continue with Facebook</button>\
             <button name=\"decision\" value=\"deny\">Cancel</button>\
             </div></form>",
            target = escape(target),
            token = escape(consent_token),
        ),
    )
}

pub fn problem(message: &str) -> String {
    layout(
        "Connection failed",
        &format!(
            "<p>{}</p><p>Start again from your MCP client.</p>",
            escape(message)
        ),
    )
}

pub fn landing(mcp_url: &str) -> String {
    layout(
        "Instagram MCP",
        &format!(
            "<p>Read-only access to your Instagram professional accounts from any MCP client: \
             profile, media, insights, hashtags, and competitor benchmarks.</p>\
             <p>Add this server URL to your MCP client:</p><p><code>{}</code></p>\
             <p><a href=\"https://github.com/mrcrgl/instagram-mcp\">Source and documentation</a></p>",
            escape(mcp_url),
        ),
    )
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_controlled_text_is_escaped() {
        let page = consent(
            Some("<script>alert(1)</script>"),
            "evil.example\"><img src=x>",
            "tok",
        );
        assert!(!page.contains("<script>"));
        assert!(!page.contains("<img"));
        assert!(page.contains("&lt;script&gt;"));
    }

    #[test]
    fn consent_names_the_target() {
        let page = consent(None, "app.example.com", "tok");
        assert!(page.contains("app.example.com"));
        assert!(page.contains("value=\"tok\""));
    }
}
