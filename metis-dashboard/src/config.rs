use metis_common::constants::ENV_METIS_API_ORIGIN;
use std::sync::OnceLock;

pub fn api_origin() -> &'static str {
    static API_ORIGIN: OnceLock<String> = OnceLock::new();
    API_ORIGIN
        .get_or_init(|| {
            let message = format!("{ENV_METIS_API_ORIGIN} must be set for the dashboard server");
            let api_origin = option_env!("METIS_API_ORIGIN")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .expect(&message);
            api_origin.to_string()
        })
        .as_str()
}
