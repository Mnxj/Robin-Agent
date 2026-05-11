#[cfg(test)]
mod tests {
    use super::super::Policy;

    #[test]
    fn test_policy_allow_all() {
        let p = Policy::default(); // empty allow/deny = allow all
        assert!(p.is_allowed("bash"));
        assert!(p.is_allowed("read_file"));
        assert!(p.is_allowed("anything"));
    }

    #[test]
    fn test_policy_allow_list() {
        let p = Policy {
            allow: vec!["read_file".to_owned(), "write_file".to_owned()],
            deny: vec![],
        };
        assert!(p.is_allowed("read_file"));
        assert!(p.is_allowed("write_file"));
        assert!(!p.is_allowed("bash"));
        assert!(!p.is_allowed("edit_file"));
    }

    #[test]
    fn test_policy_deny_list() {
        let p = Policy {
            allow: vec![],
            deny: vec!["bash".to_owned()],
        };
        assert!(!p.is_allowed("bash"));
        assert!(p.is_allowed("read_file"));
        assert!(p.is_allowed("write_file"));
    }

    #[test]
    fn test_policy_allow_and_deny() {
        let p = Policy {
            allow: vec!["read_file".to_owned(), "bash".to_owned()],
            deny: vec!["bash".to_owned()],
        };
        assert!(p.is_allowed("read_file"));
        assert!(!p.is_allowed("bash")); // denied despite being in allow
        assert!(!p.is_allowed("web_fetch")); // not in allow
    }

    #[test]
    fn test_policy_wildcard_allow() {
        let p = Policy {
            allow: vec!["*".to_owned()],
            deny: vec!["bash".to_owned()],
        };
        assert!(p.is_allowed("read_file"));
        assert!(!p.is_allowed("bash"));
    }

    #[test]
    fn test_policy_wildcard_deny() {
        let p = Policy {
            allow: vec![],
            deny: vec!["*".to_owned()],
        };
        assert!(!p.is_allowed("bash"));
        assert!(!p.is_allowed("read_file"));
    }
}