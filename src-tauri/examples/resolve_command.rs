//! Read-only probe: prints the model's selected action without executing it.
fn main() -> anyhow::Result<()> {
    let settings = vasya::config::Settings::default();
    let utterance = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Расскажи, что такое IPv6".into());
    println!(
        "{}",
        serde_json::to_string_pretty(&vasya::generator::resolve(&settings, &utterance, || false)?)?
    );
    Ok(())
}
