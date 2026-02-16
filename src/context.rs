use std::path::Path;

pub fn run(dir: &Path, query: Option<&str>, plain: bool) -> Result<(), String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    section("Topics", plain);
    crate::topics::list(dir)?;

    section("Recent (7 days)", plain);
    crate::topics::recent(dir, 7, plain)?;

    if let Some(q) = query {
        section(&format!("Search: {q}"), plain);
        crate::search::run(dir, q, plain)?;
    }

    Ok(())
}

fn section(title: &str, plain: bool) {
    if plain {
        println!("\n== {title} ==");
    } else {
        println!("\n\x1b[1;35m== {title} ==\x1b[0m");
    }
}
