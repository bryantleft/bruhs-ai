use bruhs_models::whiteboard::benchmark_scene;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let count: usize = args.next().ok_or("pass shape count")?.parse()?;
    let path = args.next().ok_or("pass output path")?;
    let scene = benchmark_scene(count)?;
    std::fs::write(path, serde_json::to_vec(&scene)?)?;
    Ok(())
}
