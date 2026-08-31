#![forbid(unsafe_code)]

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use agent_platform_api::ROUTES;

fn main() {
    println!("cargo:rerun-if-changed=website/index.html.in");
    println!("cargo:rerun-if-changed=website/api.html.in");
    println!("cargo:rerun-if-changed=website/styles.css");

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    let version = env::var("CARGO_PKG_VERSION").expect("Cargo provides package version");
    let digest = agent_platform_openapi::document_sha256();
    let rows = route_rows();

    render(
        "website/index.html.in",
        &output.join("index.html"),
        &version,
        &digest,
        &rows,
    );
    render(
        "website/api.html.in",
        &output.join("api.html"),
        &version,
        &digest,
        &rows,
    );
    fs::copy("website/styles.css", output.join("styles.css"))
        .expect("copy embedded documentation stylesheet");
}

fn render(source: &str, target: &Path, version: &str, digest: &str, rows: &str) {
    let template = fs::read_to_string(source).expect("read documentation template");
    let rendered = template
        .replace("{{VERSION}}", version)
        .replace("{{OPENAPI_SHA256}}", digest)
        .replace("{{ROUTE_ROWS}}", rows);
    fs::write(target, rendered).expect("write generated documentation page");
}

fn route_rows() -> String {
    let mut rows = String::new();
    for route in ROUTES {
        let access = if route.authenticated {
            "Bearer token"
        } else {
            "Public"
        };
        write!(
            rows,
            "<tr><td><span class=\"method method-{}\">{}</span></td><td><code>{}</code></td><td>{}</td><td>{}</td></tr>",
            route.method.as_str().to_ascii_lowercase(),
            route.method.as_str(),
            escape(route.path),
            escape(route.operation.summary()),
            access,
        )
        .expect("writing to a String cannot fail");
    }
    rows
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
