#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::config::{
        default_config, load, strip_json5, validate_reasoning_mode, AgentConfig, AgentsConfig,
        AgentsDefaults, Binding, BindingMatch, CLIConfig, ChannelsConfig, CompactionConfig,
        Config, CortexConfig, DecisionBehavior, GatewayConfig, MCPAuthConfig,
        MCPHTTPBlock, MCPServerConfig, MCPStdioBlock, MemoryConfig, OTelConfig, OTelSignals,
        ReloadConfig, SecurityConfig, ToolPolicy,
    };

    fn write_temp_config(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("robin.json5");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn test_default_config() {
        let cfg = default_config();
        assert_eq!(cfg.gateway.host, "127.0.0.1");
        assert_eq!(cfg.gateway.port, 18789);
        assert_eq!(cfg.agents.list.len(), 1);
        assert_eq!(cfg.agents.list[0].id, "default");
        assert_eq!(cfg.agents.list[0].model, "");
    }

    #[test]
    fn test_load_missing_file() {
        let cfg = load("/nonexistent/path/robin.json5").unwrap();
        assert_eq!(cfg.agents.list[0].id, "default");
    }

    #[test]
    fn test_load_json5() {
        let content = r#"{
  // This is a comment
  "gateway": {
    "host": "0.0.0.0",
    "port": 9999,
  },
  "agents": {
    "list": [
      {
        "id": "test",
        "name": "Test Agent",
        "model": "openai/gpt-4o",
      },
    ],
  },
}"#;
        let (_dir, path) = write_temp_config(content);
        let cfg = load(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.gateway.host, "0.0.0.0");
        assert_eq!(cfg.gateway.port, 9999);
        assert_eq!(cfg.agents.list[0].id, "test");
        assert_eq!(cfg.agents.list[0].model, "openai/gpt-4o");
    }

    #[test]
    fn test_validate_no_agents() {
        let mut cfg = Config::default();
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("at least one agent"));
    }

    #[test]
    fn test_validate_no_model() {
        let mut cfg = Config::default();
        cfg.agents.list = vec![AgentConfig { id: "x".to_string(), ..Default::default() }];
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("no model"));
    }

    #[test]
    fn test_get_agent() {
        let cfg = default_config();
        let (a, ok) = (cfg.get_agent("default"), true);
        assert!(a.is_some());
        assert_eq!(a.unwrap().name, "Robin");

        let missing = cfg.get_agent("nonexistent");
        assert!(missing.is_none());
    }


    #[test]
    fn test_strip_json5() {
        let cases = vec![
            (
                "strip single-line comment",
                "// comment\n{\"key\": \"value\"}",
                "{\"key\": \"value\"}\n",
            ),
            (
                "strip trailing comma before }",
                r#"{"key": "value",}"#,
                "{\"key\": \"value\"}\n",
            ),
            (
                "strip trailing comma before ]",
                r#"["a", "b",]"#,
                "[\"a\", \"b\"]\n",
            ),
        ];
        for (name, input, want) in cases {
            let got = strip_json5(input);
            assert_eq!(got, want, "case: {}", name);
        }
    }

    #[test]
    fn test_compaction_defaults_are_sensible() {
        let cfg = default_config();
        let c = &cfg.agents.defaults.compaction;
        assert!(c.enabled);
        assert!(c.model.is_empty(), "Model is empty by default");
        assert!((c.threshold - 0.6).abs() < 0.001);
        assert_eq!(c.preserve_turns, 4);
        assert_eq!(c.timeout_sec, 60);
    }

    #[test]
    fn test_compaction_config_unmarshals() {
        let raw = r#"{
            "agents": {
                "defaults": {
                    "compaction": {
                        "enabled": false,
                        "model": "local/gemma2:2b",
                        "threshold": 0.5,
                        "preserveTurns": 6,
                        "timeoutSec": 30
                    }
                }
            }
        }"#;
        let cfg: Config = serde_json::from_str(raw).unwrap();
        let c = &cfg.agents.defaults.compaction;
        assert!(!c.enabled);
        assert_eq!(c.model, "local/gemma2:2b");
        assert!((c.threshold - 0.5).abs() < 0.001);
        assert_eq!(c.preserve_turns, 6);
        assert_eq!(c.timeout_sec, 30);
    }

    #[test]
    fn test_default_config_cortex_embed_defaults() {
        let cfg = default_config();
        assert_eq!(cfg.memory.embedding_provider, "");
        assert_eq!(cfg.memory.embedding_model, "");
        assert!(cfg.memory.enabled);
        assert_eq!(cfg.cortex.provider, "", "Cortex.Provider default should be empty");
        assert_eq!(cfg.cortex.llm_model, "", "Cortex.LLMModel default should be empty");
    }

    #[test]
    fn test_validate_preserves_explicit_cortex_pin() {
        let mut cfg = default_config();
        // Give the default agent a model so validate() passes.
        if let Some(a) = cfg.agents.list.first_mut() { a.model = "anthropic/claude-sonnet-4-5".to_string(); }
        cfg.cortex.provider = "anthropic".to_string();
        cfg.cortex.llm_model = "claude-sonnet-4-6".to_string();
        cfg.validate().unwrap();
        assert_eq!(cfg.cortex.provider, "anthropic");
        assert_eq!(cfg.cortex.llm_model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_resolve_mcp_servers_happy_path() {
        std::env::set_var("LTM_SECRET_FOR_TEST", "shhh");
        let cfg = Config {
            mcp_servers: vec![
                MCPServerConfig {
                    id: "ltm".to_string(),
                    transport: "http".to_string(),
                    http: Some(MCPHTTPBlock {
                        url: "https://example.com/mcp".to_string(),
                        auth: MCPAuthConfig {
                            kind: "oauth2_client_credentials".to_string(),
                            token_url: Some("https://example.com/oauth/token".to_string()),
                            client_id: Some("client-x".to_string()),
                            client_secret_env: Some("LTM_SECRET_FOR_TEST".to_string()),
                            scope: Some("ltm/api".to_string()),
                            ..Default::default()
                        },
                    }),
                    enabled: true,
                    tool_prefix: "ltm_".to_string(),
                    ..Default::default()
                },
                MCPServerConfig { id: "disabled-one".to_string(), enabled: false, ..Default::default() },
            ],
            ..Default::default()
        };

        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "ltm");
        assert_eq!(got[0].transport, "http");
        assert!(got[0].http.is_some());
        assert_eq!(got[0].http.as_ref().unwrap().auth.client_secret, "shhh");
        assert_eq!(got[0].tool_prefix, "ltm_");
    }

    #[test]
    fn test_resolve_mcp_servers_literal_secret_in_config() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "ltm".to_string(), transport: "http".to_string(), enabled: true,
                http: Some(MCPHTTPBlock {
                    url: "https://x".to_string(),
                    auth: MCPAuthConfig {
                        kind: "oauth2_client_credentials".to_string(),
                        token_url: Some("https://t".to_string()),
                        client_id: Some("c".to_string()),
                        client_secret: Some("literal-secret".to_string()),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].http.as_ref().unwrap().auth.client_secret, "literal-secret");
    }

    #[test]
    fn test_resolve_mcp_servers_literal_beats_env() {
        std::env::set_var("SECRET_THAT_SHOULD_NOT_WIN", "from-env");
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "ltm".to_string(), transport: "http".to_string(), enabled: true,
                http: Some(MCPHTTPBlock {
                    url: "https://x".to_string(),
                    auth: MCPAuthConfig {
                        kind: "oauth2_client_credentials".to_string(),
                        token_url: Some("https://t".to_string()),
                        client_id: Some("c".to_string()),
                        client_secret: Some("from-config".to_string()),
                        client_secret_env: Some("SECRET_THAT_SHOULD_NOT_WIN".to_string()),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].http.as_ref().unwrap().auth.client_secret, "from-config");
    }

    #[test]
    fn test_resolve_mcp_servers_missing_secret_skips_server() {
        let cfg = Config {
            mcp_servers: vec![
                MCPServerConfig {
                    id: "ltm-bad".to_string(), transport: "http".to_string(), enabled: true,
                    http: Some(MCPHTTPBlock {
                        url: "https://x".to_string(),
                        auth: MCPAuthConfig {
                            kind: "oauth2_client_credentials".to_string(),
                            token_url: Some("https://t".to_string()),
                            client_id: Some("c".to_string()),
                            client_secret_env: Some("DEFINITELY_NOT_SET_ROBIN_TEST".to_string()),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
                MCPServerConfig {
                    id: "ltm-good".to_string(), transport: "http".to_string(), enabled: true,
                    http: Some(MCPHTTPBlock {
                        url: "https://y".to_string(),
                        auth: MCPAuthConfig {
                            kind: "oauth2_client_credentials".to_string(),
                            token_url: Some("https://t".to_string()),
                            client_id: Some("c".to_string()),
                            client_secret: Some("ok".to_string()),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "ltm-good");
    }

    #[test]
    fn test_resolve_mcp_servers_empty_id_skips_server() {
        let cfg = Config {
            mcp_servers: vec![
                MCPServerConfig { id: "".to_string(), transport: "http".to_string(), enabled: true, ..Default::default() },
                MCPServerConfig {
                    id: "ltm-good".to_string(), transport: "http".to_string(), enabled: true,
                    http: Some(MCPHTTPBlock {
                        url: "https://y".to_string(),
                        auth: MCPAuthConfig {
                            kind: "oauth2_client_credentials".to_string(),
                            token_url: Some("https://t".to_string()),
                            client_id: Some("c".to_string()),
                            client_secret: Some("ok".to_string()),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "ltm-good");
    }

    #[test]
    fn test_resolve_mcp_servers_unsupported_auth_kind() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "ltm".to_string(), transport: "http".to_string(), enabled: true,
                http: Some(MCPHTTPBlock {
                    url: "https://x".to_string(),
                    auth: MCPAuthConfig { kind: "weird-scheme".to_string(), ..Default::default() },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = cfg.resolve_mcp_servers().unwrap_err();
        assert!(err.to_string().contains("unsupported auth.kind"));
    }

    #[test]
    fn test_resolve_mcp_servers_bearer_literal() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "anthropic".to_string(), transport: "http".to_string(), enabled: true,
                http: Some(MCPHTTPBlock {
                    url: "https://mcp.anthropic.com/v1/x".to_string(),
                    auth: MCPAuthConfig {
                        kind: "bearer".to_string(),
                        token: Some("sk-ant-literal".to_string()),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].http.as_ref().unwrap().auth.kind, "bearer");
        assert_eq!(got[0].http.as_ref().unwrap().auth.bearer_token, "sk-ant-literal");
    }

    #[test]
    fn test_resolve_mcp_servers_bearer_env() {
        std::env::set_var("BEARER_FOR_TEST", "from-env-tok");
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "x".to_string(), transport: "http".to_string(), enabled: true,
                http: Some(MCPHTTPBlock {
                    url: "https://x".to_string(),
                    auth: MCPAuthConfig {
                        kind: "bearer".to_string(),
                        token_env: Some("BEARER_FOR_TEST".to_string()),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].http.as_ref().unwrap().auth.bearer_token, "from-env-tok");
    }

    #[test]
    fn test_resolve_mcp_servers_bearer_missing_token_skips() {
        let cfg = Config {
            mcp_servers: vec![
                MCPServerConfig {
                    id: "no-token".to_string(), transport: "http".to_string(), enabled: true,
                    http: Some(MCPHTTPBlock {
                        url: "https://x".to_string(),
                        auth: MCPAuthConfig {
                            kind: "bearer".to_string(),
                            token_env: Some("DEFINITELY_NOT_SET_BEARER_TEST".to_string()),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
                MCPServerConfig {
                    id: "ok".to_string(), transport: "http".to_string(), enabled: true,
                    http: Some(MCPHTTPBlock {
                        url: "https://y".to_string(),
                        auth: MCPAuthConfig {
                            kind: "bearer".to_string(),
                            token: Some("tok".to_string()),
                            ..Default::default()
                        },
                    }),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "ok");
    }

    #[test]
    fn test_resolve_mcp_servers_none_auth() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "local-mcp".to_string(), transport: "http".to_string(), enabled: true,
                http: Some(MCPHTTPBlock {
                    url: "http://127.0.0.1:9999/mcp".to_string(),
                    auth: MCPAuthConfig { kind: "none".to_string(), ..Default::default() },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].http.as_ref().unwrap().auth.kind, "none");
    }

    #[test]
    fn test_resolve_mcp_servers_stdio() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "github".to_string(), transport: "stdio".to_string(), enabled: true,
                stdio: Some(MCPStdioBlock {
                    command: "npx".to_string(),
                    args: vec!["-y".to_string(), "@modelcontextprotocol/server-github".to_string()],
                    env: [("GITHUB_TOKEN".to_string(), "ghp_xxx".to_string())].into_iter().collect(),
                }),
                tool_prefix: "gh_".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].transport, "stdio");
        assert!(got[0].stdio.is_some());
        let stdio = got[0].stdio.as_ref().unwrap();
        assert_eq!(stdio.command, "npx");
        assert_eq!(stdio.args, vec!["-y", "@modelcontextprotocol/server-github"]);
        assert_eq!(stdio.env.get("GITHUB_TOKEN").unwrap(), "ghp_xxx");
        assert_eq!(got[0].tool_prefix, "gh_");
    }

    #[test]
    fn test_resolve_mcp_servers_stdio_missing_command() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "broken".to_string(), transport: "stdio".to_string(), enabled: true,
                stdio: Some(MCPStdioBlock { args: vec!["x".to_string()], ..Default::default() }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = cfg.resolve_mcp_servers().unwrap_err();
        assert!(err.to_string().contains("stdio.command"));
    }

    #[test]
    fn test_resolve_mcp_servers_legacy_flat_http() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "ltm-legacy".to_string(),
                url: "https://legacy.example.com/mcp".to_string(),
                enabled: true,
                auth: MCPAuthConfig {
                    kind: "oauth2_client_credentials".to_string(),
                    token_url: Some("https://t".to_string()),
                    client_id: Some("c".to_string()),
                    client_secret: Some("legacy-sec".to_string()),
                    scope: Some("x".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].transport, "http");
        assert!(got[0].http.is_some());
        assert_eq!(got[0].http.as_ref().unwrap().url, "https://legacy.example.com/mcp");
        assert_eq!(got[0].http.as_ref().unwrap().auth.client_secret, "legacy-sec");
    }

    #[test]
    fn test_resolve_mcp_servers_nested_wins_over_flat() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "x".to_string(), transport: "http".to_string(), enabled: true,
                url: "https://flat.example.com".to_string(),
                auth: MCPAuthConfig {
                    kind: "oauth2_client_credentials".to_string(),
                    token_url: Some("https://t".to_string()),
                    client_id: Some("c".to_string()),
                    client_secret: Some("flat-sec".to_string()),
                    ..Default::default()
                },
                http: Some(MCPHTTPBlock {
                    url: "https://nested.example.com".to_string(),
                    auth: MCPAuthConfig {
                        kind: "oauth2_client_credentials".to_string(),
                        token_url: Some("https://t".to_string()),
                        client_id: Some("c".to_string()),
                        client_secret: Some("nested-sec".to_string()),
                        ..Default::default()
                    },
                }),
                ..Default::default()
            }],
            ..Default::default()
        };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].http.as_ref().unwrap().url, "https://nested.example.com");
        assert_eq!(got[0].http.as_ref().unwrap().auth.client_secret, "nested-sec");
    }

    #[test]
    fn test_resolve_mcp_servers_unknown_transport() {
        let cfg = Config {
            mcp_servers: vec![MCPServerConfig {
                id: "x".to_string(), transport: "carrier-pigeon".to_string(), enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let err = cfg.resolve_mcp_servers().unwrap_err();
        assert!(err.to_string().contains("unsupported transport"));
    }

    #[test]
    fn test_resolve_mcp_servers_round_trip_json_stdio() {
        let original = MCPServerConfig {
            id: "rt".to_string(), transport: "stdio".to_string(), enabled: true,
            stdio: Some(MCPStdioBlock {
                command: "echo".to_string(),
                args: vec!["hi".to_string()],
                env: [("K".to_string(), "V".to_string())].into_iter().collect(),
            }),
            ..Default::default()
        };
        let data = serde_json::to_string(&original).unwrap();
        let parsed: MCPServerConfig = serde_json::from_str(&data).unwrap();
        let cfg = Config { mcp_servers: vec![parsed], ..Default::default() };
        let got = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].stdio.as_ref().unwrap().command, "echo");
        assert_eq!(got[0].stdio.as_ref().unwrap().args, vec!["hi"]);
        assert_eq!(got[0].stdio.as_ref().unwrap().env.get("K").unwrap(), "V");
    }

    #[test]
    fn test_apply_mcp_tool_names_to_allowlists() {
        let mut cfg = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig { id: "with-allowlist".to_string(), tools: ToolPolicy { allow: vec!["bash".to_string(), "read_file".to_string()], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "wide-open".to_string(), tools: ToolPolicy { allow: vec![], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "empty-allow".to_string(), tools: ToolPolicy { allow: vec![], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "already-has-one".to_string(), tools: ToolPolicy { allow: vec!["bash".to_string(), "ltm_search".to_string()], deny: vec![] }, ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.apply_mcp_tool_names_to_allowlists(&["ltm_search".to_string(), "ltm_store".to_string()]);

        let list = &cfg.agents.list;
        let mut a0 = list[0].tools.allow.clone(); a0.sort();
        let mut expected0 = vec!["bash", "ltm_search", "ltm_store", "read_file"]; expected0.sort();
        assert_eq!(a0, expected0);

        assert!(list[1].tools.allow.is_empty(), "wide-open agent should be left alone");
        assert!(list[2].tools.allow.is_empty(), "empty-allow agent should be left alone");

        let mut a3 = list[3].tools.allow.clone(); a3.sort();
        let mut expected3 = vec!["bash", "ltm_search", "ltm_store"]; expected3.sort();
        assert_eq!(a3, expected3, "duplicate ltm_search should not appear twice");
    }

    #[test]
    fn test_apply_mcp_tool_names_to_allowlists_empty() {
        let mut cfg = Config {
            agents: AgentsConfig {
                list: vec![AgentConfig {
                    id: "x".to_string(),
                    tools: ToolPolicy { allow: vec!["bash".to_string()], deny: vec![] },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.apply_mcp_tool_names_to_allowlists(&[]);
        assert_eq!(cfg.agents.list[0].tools.allow, vec!["bash"]);
    }

    #[test]
    fn test_strip_mcp_auto_added() {
        let mut runtime = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig { id: "with-allow".to_string(), tools: ToolPolicy { allow: vec!["bash".to_string()], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "wide-open".to_string(), tools: ToolPolicy { allow: vec![], deny: vec![] }, ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        runtime.apply_mcp_tool_names_to_allowlists(&["ltm_x".to_string(), "ltm_y".to_string()]);
        let a0 = &runtime.agents.list[0].tools.allow;
        assert!(a0.contains(&"ltm_x".to_string()) && a0.contains(&"ltm_y".to_string()));

        let mut incoming = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig { id: "with-allow".to_string(), tools: ToolPolicy { allow: vec!["bash".to_string(), "ltm_x".to_string(), "ltm_y".to_string()], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "wide-open".to_string(), tools: ToolPolicy { allow: vec![], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "newcomer".to_string(), tools: ToolPolicy { allow: vec!["web_fetch".to_string(), "ltm_x".to_string()], deny: vec![] }, ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        runtime.strip_mcp_auto_added(&mut incoming);
        assert_eq!(incoming.agents.list[0].tools.allow, vec!["bash"]);
        assert!(incoming.agents.list[1].tools.allow.is_empty());
        assert_eq!(incoming.agents.list[2].tools.allow, vec!["web_fetch"]);
    }

    #[test]
    fn test_strip_mcp_auto_added_no_snapshot() {
        let runtime = Config::default();
        let mut incoming = Config {
            agents: AgentsConfig {
                list: vec![AgentConfig {
                    id: "x".to_string(),
                    tools: ToolPolicy { allow: vec!["bash".to_string(), "ltm_x".to_string()], deny: vec![] },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        runtime.strip_mcp_auto_added(&mut incoming);
        let mut got = incoming.agents.list[0].tools.allow.clone(); got.sort();
        let mut expected = vec!["bash", "ltm_x"]; expected.sort();
        assert_eq!(got, expected);
    }

    #[test]
    fn test_apply_task_tool_to_allowlists() {
        let mut cfg = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig { id: "parent".to_string(), model: "x/y".to_string(), tools: ToolPolicy { allow: vec!["read_file".to_string(), "bash".to_string()], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "researcher".to_string(), model: "x/y".to_string(), subagent: true, description: "Web".to_string(), tools: ToolPolicy { allow: vec!["web_fetch".to_string()], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "free".to_string(), model: "x/y".to_string(), ..Default::default() },
                    AgentConfig { id: "already_has".to_string(), model: "x/y".to_string(), tools: ToolPolicy { allow: vec!["read_file".to_string(), "task".to_string()], deny: vec![] }, ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.apply_task_tool_to_allowlists();

        assert!(cfg.agents.list[0].tools.allow.contains(&"task".to_string()), "parent should gain task");
        assert!(cfg.agents.list[1].tools.allow.contains(&"task".to_string()), "researcher should gain task");
        assert!(cfg.agents.list[2].tools.allow.is_empty(), "free agent (empty allow) should be untouched");
        assert_eq!(cfg.agents.list[3].tools.allow, vec!["read_file", "task"], "no duplicate task");
    }

    #[test]
    fn test_apply_task_tool_to_allowlists_no_subagents() {
        let mut cfg = Config {
            agents: AgentsConfig {
                list: vec![AgentConfig {
                    id: "parent".to_string(), model: "x/y".to_string(),
                    tools: ToolPolicy { allow: vec!["read_file".to_string()], deny: vec![] },
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.apply_task_tool_to_allowlists();
        assert_eq!(cfg.agents.list[0].tools.allow, vec!["read_file"]);
    }

    #[test]
    fn test_apply_task_tool_to_allowlists_idempotent() {
        let mut cfg = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig { id: "parent".to_string(), model: "x/y".to_string(), tools: ToolPolicy { allow: vec!["read_file".to_string()], deny: vec![] }, ..Default::default() },
                    AgentConfig { id: "sub".to_string(), model: "x/y".to_string(), subagent: true, description: "x".to_string(), ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.apply_task_tool_to_allowlists();
        cfg.apply_task_tool_to_allowlists();
        assert_eq!(cfg.agents.list[0].tools.allow, vec!["read_file", "task"]);
    }

    #[test]
    fn test_agent_loop_defaults_to_zero() {
        let cfg = default_config();
        assert_eq!(cfg.agent_loop.max_tool_concurrency, 0);
        assert_eq!(cfg.agent_loop.max_agent_depth, 0);
        assert!(!cfg.agent_loop.streaming_tools);
    }

    #[test]
    fn test_agent_loop_unmarshals_explicit_values() {
        let raw = r#"{
            "agents": { "list": [{"id": "a", "model": "x/y"}] },
            "agentLoop": {
                "maxToolConcurrency": 4,
                "maxAgentDepth": 7,
                "streamingTools": true
            }
        }"#;
        let cfg: Config = serde_json::from_str(raw).unwrap();
        assert_eq!(cfg.agent_loop.max_tool_concurrency, 4);
        assert_eq!(cfg.agent_loop.max_agent_depth, 7);
        assert!(cfg.agent_loop.streaming_tools);
    }

    #[test]
    fn test_agent_loop_load_from_json5_file() {
        let contents = r#"{
            "agents": { "list": [{"id": "a", "model": "x/y"}] },
            "agentLoop": {
                // in-line comment is fine — JSON5 path
                "maxToolConcurrency": 12,
                "maxAgentDepth": 5,
                "streamingTools": true,
            },
        }"#;
        let (_dir, path) = write_temp_config(contents);
        let cfg = load(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.agent_loop.max_tool_concurrency, 12);
        assert_eq!(cfg.agent_loop.max_agent_depth, 5);
        assert!(cfg.agent_loop.streaming_tools);
    }

    #[test]
    fn test_update_from_copies_agent_loop() {
        let mut dst = Config::default();
        let src = Config {
            agents: AgentsConfig {
                list: vec![AgentConfig { id: "a".to_string(), model: "x/y".to_string(), ..Default::default() }],
                ..Default::default()
            },
            agent_loop: super::super::config::AgentLoopConfig {
                max_tool_concurrency: 7,
                max_agent_depth: 9,
                streaming_tools: true,
            },
            ..Default::default()
        };
        dst.update_from(&src);
        assert_eq!(dst.agent_loop.max_tool_concurrency, 7);
        assert_eq!(dst.agent_loop.max_agent_depth, 9);
        assert!(dst.agent_loop.streaming_tools);
    }

    #[test]
    fn test_compaction_config_message_cap_default() {
        let cfg = default_config();
        assert_eq!(cfg.agents.defaults.compaction.message_cap, 50);
    }

    #[test]
    fn test_compaction_config_message_cap_zero_disables_cap() {
        let mut cfg = CompactionConfig::default();
        cfg.message_cap = 0;
        assert_eq!(cfg.message_cap, 0);
    }

    #[test]
    fn test_agent_config_reasoning_validation() {
        let cases = vec![
            ("", true),
            ("off", true),
            ("low", true),
            ("medium", true),
            ("high", true),
            ("ultra", false),
            ("LOW", false),
        ];
        for (input, want_ok) in cases {
            let result = validate_reasoning_mode(input);
            if want_ok {
                assert!(result.is_ok(), "input {:?} should validate", input);
            } else {
                assert!(result.is_err(), "input {:?} should error", input);
            }
        }
    }

    #[test]
    fn test_subagent_requires_description() {
        let mut cfg = Config {
            agents: AgentsConfig {
                list: vec![AgentConfig {
                    id: "worker".to_string(),
                    model: "openai/gpt-4o".to_string(),
                    subagent: true,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("worker"));
        assert!(err.to_string().contains("description"));

        cfg.agents.list[0].description = "Web research subagent".to_string();
        cfg.validate().unwrap();
    }

    #[test]
    fn test_eligible_subagents() {
        let cfg = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig { id: "worker".to_string(), model: "openai/gpt-4o".to_string(), subagent: true, description: "Web research".to_string(), ..Default::default() },
                    AgentConfig { id: "summarizer".to_string(), model: "openai/gpt-4o".to_string(), subagent: true, description: "Summarizes long text".to_string(), ..Default::default() },
                    AgentConfig { id: "default".to_string(), model: "openai/gpt-4o".to_string(), subagent: false, ..Default::default() },
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let got = cfg.eligible_subagents();
        assert_eq!(got.get("worker").unwrap(), "Web research");
        assert_eq!(got.get("summarizer").unwrap(), "Summarizes long text");
        assert!(!got.contains_key("default"));
    }

    #[test]
    fn test_eligible_subagents_none_eligible() {
        let cfg = Config {
            agents: AgentsConfig {
                list: vec![AgentConfig { id: "default".to_string(), model: "openai/gpt-4o".to_string(), ..Default::default() }],
                ..Default::default()
            },
            ..Default::default()
        };
        let got = cfg.eligible_subagents();
        assert_eq!(got.len(), 0);
    }

    #[test]
    fn test_build_permission_checker() {
        let cfg = Config {
            agents: AgentsConfig {
                list: vec![
                    AgentConfig {
                        id: "agent_allow".to_string(),
                        tools: ToolPolicy { allow: vec!["read_file".to_string(), "web_fetch".to_string()], deny: vec![] },
                        ..Default::default()
                    },
                    AgentConfig {
                        id: "agent_deny".to_string(),
                        tools: ToolPolicy { allow: vec![], deny: vec!["bash".to_string()] },
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let checker = cfg.build_permission_checker();

        assert_eq!(checker.check("agent_allow", "read_file", b"{}").behavior, DecisionBehavior::Allow);
        assert_eq!(checker.check("agent_allow", "bash", b"{}").behavior, DecisionBehavior::Deny);

        assert_eq!(checker.check("agent_deny", "bash", b"{}").behavior, DecisionBehavior::Deny);
        assert_eq!(checker.check("agent_deny", "read_file", b"{}").behavior, DecisionBehavior::Allow);

        assert_eq!(checker.check("unknown", "anything", b"{}").behavior, DecisionBehavior::Allow);
    }

    #[test]
    fn test_mcp_server_parallel_safe_round_trips_through_json5() {
        let contents = r#"{
            "agents": { "list": [{"id": "a", "model": "x/y"}] },
            "mcp_servers": [{
                "id": "trusted",
                "enabled": true,
                "parallelSafe": true,
                "transport": "http",
                "http": { "url": "http://example.com" }
            }],
        }"#;
        let (_dir, path) = write_temp_config(contents);
        let cfg = load(path.to_str().unwrap()).unwrap();
        assert_eq!(cfg.mcp_servers.len(), 1);
        assert!(cfg.mcp_servers[0].parallel_safe);

        let resolved = cfg.resolve_mcp_servers().unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].parallel_safe);
    }

    #[test]
    fn test_is_server_parallel_safe() {
        let cfg = Config {
            mcp_servers: vec![
                MCPServerConfig { id: "trusted".to_string(), parallel_safe: true, ..Default::default() },
                MCPServerConfig { id: "default".to_string(), parallel_safe: false, ..Default::default() },
            ],
            ..Default::default()
        };
        assert!(cfg.is_server_parallel_safe("trusted"));
        assert!(!cfg.is_server_parallel_safe("default"));
        assert!(!cfg.is_server_parallel_safe("missing"));
    }

    #[test]
    fn test_is_server_parallel_safe_updates_after_hot_reload() {
        let mut cfg = Config {
            mcp_servers: vec![MCPServerConfig { id: "trusted".to_string(), parallel_safe: false, ..Default::default() }],
            ..Default::default()
        };
        assert!(!cfg.is_server_parallel_safe("trusted"));

        let src = Config {
            mcp_servers: vec![MCPServerConfig { id: "trusted".to_string(), parallel_safe: true, ..Default::default() }],
            agents: AgentsConfig {
                list: vec![AgentConfig { id: "a".to_string(), model: "x/y".to_string(), ..Default::default() }],
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.update_from(&src);
        assert!(cfg.is_server_parallel_safe("trusted"));
    }
}