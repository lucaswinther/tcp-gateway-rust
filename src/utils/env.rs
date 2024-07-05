pub fn load_env() -> (String, String) {
    let redis_host = std::env::var("REDIS_HOST").expect("REDIS_HOST must be set");
    let redis_key = std::env::var("REDIS_KEY").unwrap_or_else(|_| "gateway_config".to_string());
    
    (redis_host, redis_key)
}
