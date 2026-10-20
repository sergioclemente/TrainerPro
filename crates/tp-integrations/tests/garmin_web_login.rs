use reqwest::Url;
use tp_integrations::garmin::GarminClient;

#[test]
fn embedded_login_keeps_ticket_service_and_returns_to_exact_parent() {
    for parent in [
        "https://training.example",
        "http://localhost:8765",
        "http://127.0.0.1:8765",
        "http://127.0.0.2:8765",
        "http://127.255.255.254:8765",
        "http://[::1]:8765",
    ] {
        let url = Url::parse(&GarminClient::embedded_sign_in_url(parent).unwrap()).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(params["source"], parent);
        assert_eq!(params["service"], "https://sso.garmin.com/sso/embed");
        assert_eq!(params["consumeServiceTicket"], "false");
        assert!(!params.contains_key("redirectAfterAccountLoginUrl"));
    }
}

#[test]
fn embedded_login_rejects_insecure_remote_and_non_origin_sources() {
    for parent in [
        "http://training.example",
        "http://126.255.255.255:8765",
        "http://128.0.0.1:8765",
        "http://192.168.1.1:8765",
        "http://127.0.0.1.example:8765",
        "http://[::2]:8765",
        "http://[::ffff:127.0.0.1]:8765",
        "https://user:password@example.com",
        "https://example.com/path",
        "https://example.com?ticket=secret",
        "https://example.com#fragment",
        "file:///tmp/login",
    ] {
        assert!(
            GarminClient::embedded_sign_in_url(parent).is_err(),
            "{parent}"
        );
    }
}
