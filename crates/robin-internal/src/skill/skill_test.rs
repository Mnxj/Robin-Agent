#[cfg(test)]
mod tests {
    use crate::skill::skill::{
        format_for_prompt, missing_bins, parse_skill_file, split_frontmatter, Loader, Skill,
    };
    use tempfile::TempDir;

    #[test]
    fn test_split_frontmatter_with_frontmatter() {
        let (fm, body) = split_frontmatter("---\nname: test\n---\n# Body\nContent here");
        assert_eq!(fm, "name: test");
        assert_eq!(body, "# Body\nContent here");
    }

    #[test]
    fn test_split_frontmatter_no_frontmatter() {
        let (fm, body) = split_frontmatter("# Just a heading\nSome content");
        assert_eq!(fm, "");
        assert_eq!(body, "# Just a heading\nSome content");
    }

    #[test]
    fn test_split_frontmatter_empty() {
        let (fm, body) = split_frontmatter("");
        assert_eq!(fm, "");
        assert_eq!(body, "");
    }

    #[test]
    fn test_split_frontmatter_only_frontmatter() {
        let (fm, body) = split_frontmatter("---\nname: test\n---\n");
        assert_eq!(fm, "name: test");
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_skill_file() {
        let dir = TempDir::new().unwrap();
        let skill_dir = dir.path().join("web-search");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = "---\nname: web-search\ndescription: Search the web for current information\ntags:\n  - search\n  - web\n  - internet\n---\n\n# Web Search Skill\n\nWhen the user asks about current events use the web_search tool.\n\n## Usage Guidelines\n- Keep queries concise\n";
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, content).unwrap();
        let skill = parse_skill_file(skill_path.to_str().unwrap()).unwrap();
        assert_eq!(skill.name, "web-search");
        assert_eq!(skill.description, "Search the web for current information");
        assert_eq!(skill.tags, vec!["search", "web", "internet"]);
        assert!(skill.body.contains("Web Search Skill"));
    }

    #[test]
    fn test_loader_load_from() {
        let dir = TempDir::new().unwrap();
        for name in ["skill-a", "skill-b"] {
            let skill_dir = dir.path().join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            let content = format!("---\nname: {name}\ndescription: Test skill {name}\n---\n\nBody of {name}");
            std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
        }
        let loader = Loader::new();
        let dirs = [dir.path().to_str().unwrap()];
        loader.load_from(&dirs).unwrap();
        assert_eq!(loader.skills().len(), 2);
    }

    #[test]
    fn test_loader_load_from_direct_md_default_name() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("my-skill.md"), "---\ndescription: A test skill\n---\n\nBody here").unwrap();
        let loader = Loader::new();
        let dirs = [dir.path().to_str().unwrap()];
        loader.load_from(&dirs).unwrap();
        let skills = loader.skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "my-skill");
    }

    #[test]
    fn test_loader_skips_missing_binary() {
        let dir = TempDir::new().unwrap();
        let fake = "---\nname: fake-tool\ndescription: Requires a nonexistent binary\nmetadata:\n  openclaw:\n    requires:\n      bins: [\"this-binary-does-not-exist-xyz\"]\n---\n\nBody here\n";
        std::fs::write(dir.path().join("fake-tool.md"), fake).unwrap();
        std::fs::write(dir.path().join("simple.md"), "---\nname: simple\n---\nBody").unwrap();
        let loader = Loader::new();
        let dirs = [dir.path().to_str().unwrap()];
        loader.load_from(&dirs).unwrap();
        let skills = loader.skills();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "simple");
    }

    #[test]
    fn test_missing_bins_none() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let s = Skill { name: "foo".into(), ..Default::default() };
        assert!(missing_bins(&s).is_empty());
    }

    #[test]
    fn test_missing_bins_present() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Skill::default();
        s.metadata.openclaw.requires.bins = vec!["sh".to_string()];
        assert!(missing_bins(&s).is_empty());
    }

    #[test]
    fn test_missing_bins_absent() {
        let _g = crate::TEST_PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut s = Skill::default();
        s.metadata.openclaw.requires.bins = vec!["definitely-not-installed-xyz-123".to_string()];
        let got = missing_bins(&s);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0], "definitely-not-installed-xyz-123");
    }

    #[test]
    fn test_loader_load_from_nonexistent() {
        let loader = Loader::new();
        loader.load_from(&["/nonexistent/path"]).unwrap();
        assert!(loader.skills().is_empty());
    }

    #[test]
    fn test_match_skills() {
        let loader = Loader::new();
        {
            let mut skills = loader.skills.write();
            *skills = vec![
                Skill { name: "web-search".into(), description: "Search the web for current information".into(), tags: vec!["search".into(), "web".into()], ..Default::default() },
                Skill { name: "calendar".into(), description: "Manage calendar events and appointments".into(), tags: vec!["calendar".into(), "schedule".into()], ..Default::default() },
                Skill { name: "code-review".into(), description: "Review code for bugs and improvements".into(), tags: vec!["code".into(), "review".into()], ..Default::default() },
            ];
        }
        let matches = loader.match_skills("search the web for latest news", 3);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "web-search");

        let matches = loader.match_skills("what's on my calendar today?", 3);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].name, "calendar");

        let matches = loader.match_skills("hello there", 3);
        assert!(matches.is_empty());
    }

    #[test]
    fn test_format_for_prompt() {
        let skills = vec![Skill { name: "test-skill".into(), body: "This is the body.".into(), ..Default::default() }];
        let result = format_for_prompt(&skills);
        assert!(result.contains("## Available Skills"));
        assert!(result.contains("### test-skill"));
        assert!(result.contains("This is the body."));
    }

    #[test]
    fn test_format_for_prompt_empty() {
        assert_eq!(format_for_prompt(&[]), "");
    }

    #[test]
    fn test_format_index() {
        let loader = Loader::new();
        {
            let mut skills = loader.skills.write();
            *skills = vec![
                Skill { name: "pdftotext".into(), description: "Extract plain text from PDF files".into(), ..Default::default() },
                Skill { name: "ffmpeg".into(), description: "Process audio and video".into(), ..Default::default() },
                Skill { name: "noDesc".into(), ..Default::default() },
            ];
        }
        let got = loader.format_index();
        assert!(got.contains("## Skills Index"));
        assert!(got.contains("**pdftotext**"));
        assert!(got.contains("Extract plain text from PDF files"));
        assert!(got.contains("**ffmpeg**"));
        assert!(got.contains("**noDesc**"));
    }

    #[test]
    fn test_format_index_empty() {
        let loader = Loader::new();
        assert_eq!(loader.format_index(), "");
    }
}