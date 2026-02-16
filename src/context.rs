use std::fmt::Write;
use std::path::Path;

pub fn run(dir: &Path, query: Option<&str>, plain: bool) -> Result<String, String> {
    run_inner(dir, query, plain, false)
}

pub fn run_brief(dir: &Path, query: Option<&str>, plain: bool) -> Result<String, String> {
    run_inner(dir, query, plain, true)
}

fn run_inner(dir: &Path, query: Option<&str>, plain: bool, brief: bool) -> Result<String, String> {
    if !dir.exists() {
        return Err(format!("{} not found", dir.display()));
    }

    let mut out = String::new();

    section(&mut out, "Topics", plain);
    if brief {
        out.push_str(&crate::topics::list_compact(dir)?);
    } else {
        out.push_str(&crate::topics::list(dir)?);
    }

    if !brief {
        section(&mut out, "Recent (7 days)", plain);
        out.push_str(&crate::topics::recent(dir, 7, plain)?);
    }

    if let Some(q) = query {
        if brief {
            section(&mut out, &format!("Search: {q}"), plain);
            let f = crate::search::Filter::none();
            out.push_str(&crate::search::run_brief(dir, q, None, &f)?);
        } else {
            section(&mut out, &format!("Search: {q}"), plain);
            let f = crate::search::Filter::none();
            out.push_str(&crate::search::run(dir, q, plain, None, &f)?);
        }
    }

    Ok(out)
}

fn section(out: &mut String, title: &str, plain: bool) {
    if plain {
        let _ = writeln!(out, "\n== {title} ==");
    } else {
        let _ = writeln!(out, "\n\x1b[1;35m== {title} ==\x1b[0m");
    }
}
