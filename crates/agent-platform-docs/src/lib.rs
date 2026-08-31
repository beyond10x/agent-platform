#![forbid(unsafe_code)]

pub struct EmbeddedAsset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
    pub cache_control: &'static str,
}

const INDEX: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/index.html"));
const API: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/api.html"));
const STYLES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/styles.css"));

pub fn asset(path: &str) -> Option<EmbeddedAsset> {
    match path {
        "index" => Some(EmbeddedAsset {
            bytes: INDEX,
            content_type: "text/html; charset=utf-8",
            cache_control: "public, max-age=300",
        }),
        "api" => Some(EmbeddedAsset {
            bytes: API,
            content_type: "text/html; charset=utf-8",
            cache_control: "public, max-age=300",
        }),
        "styles" => Some(EmbeddedAsset {
            bytes: STYLES,
            content_type: "text/css; charset=utf-8",
            cache_control: "public, max-age=3600",
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use agent_platform_api::ROUTES;

    use super::*;

    #[test]
    fn embedded_pages_name_the_generated_contract_and_every_api_route() {
        let index = String::from_utf8(INDEX.to_vec()).unwrap();
        let api = String::from_utf8(API.to_vec()).unwrap();
        assert!(index.contains("/openapi.json"));
        assert!(api.contains(&agent_platform_openapi::document_sha256()));
        for route in ROUTES {
            assert!(api.contains(route.path));
        }
    }

    #[test]
    fn only_curated_assets_are_addressable() {
        assert!(asset("index").is_some());
        assert!(asset("api").is_some());
        assert!(asset("styles").is_some());
        assert!(asset("../../planning").is_none());
    }
}
