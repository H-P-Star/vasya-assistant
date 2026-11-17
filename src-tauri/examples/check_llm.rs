fn main() -> anyhow::Result<()> {
    let settings = vasya::config::Settings::default();
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Объясни, что такое IPv6, одним предложением.".into());
    println!("{} / {}", settings.llm_url, settings.llm_model);
    println!("{}", vasya::generator::generate(&settings, &prompt)?);
    Ok(())
}
