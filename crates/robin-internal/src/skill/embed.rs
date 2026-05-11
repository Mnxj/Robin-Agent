use tracing::info;

/// Bundled starter skills embedded at compile time.
/// Mirrors Go's //go:embed bundled/*.md
static BUNDLED_SKILLS: &[(&str, &str)] = &[
    ("cortex.md",      include_str!("bundled/cortex.md")),
    ("ffmpeg.md",      include_str!("bundled/ffmpeg.md")),
    ("imagemagick.md", include_str!("bundled/imagemagick.md")),
    ("pandoc.md",      include_str!("bundled/pandoc.md")),
    ("pdftotext.md",   include_str!("bundled/pdftotext.md")),
];

/// Write bundled starter skills into dir, but only if dir is empty.
/// Mirrors Go's SeedBundledSkills.
pub fn seed_bundled_skills(dir: &str) -> anyhow::Result<Vec<String>> {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("read skills dir: {}", e))?
        .collect();
    if !entries.is_empty() {
        return Ok(Vec::new());
    }
    let mut written = Vec::new();
    for (name, content) in BUNDLED_SKILLS {
        let dst = std::path::Path::new(dir).join(name);
        std::fs::write(&dst, content)
            .map_err(|e| anyhow::anyhow!("write {:?}: {}", dst, e))?;
        written.push(name.to_string());
    }
    if !written.is_empty() {
        info!(dir = dir, count = written.len(), names = ?written, "seeded bundled skills");
    }
    Ok(written)
}