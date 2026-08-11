/// One-shot logger init shared by every cdylib here (the PE mods and the unix `.so`).
///
/// Each cdylib has its own copy of the `log` / `env_logger` statics, so each
/// calls this from its own entry point (`DllMain` on PE, the `InitLogger`
/// thunk on the unix side). `try_init` is idempotent so repeat calls silently
/// no-op.
///
/// Forces `WriteStyle::Always` on every target: the PE side runs inside
/// Wine where stderr-is-a-TTY auto-detection returns false even when
/// macOS fd 2 is a real terminal, and we want consistent colour across
/// all three linkage units. `NO_COLOR=1` opts out — `env_logger` only reads
/// it under `WriteStyle::Auto`, so we have to pre-resolve the choice here.
pub fn init_logger() {
    let user = std::env::var("RUST_LOG").ok();
    let filter = resolved_log_filter(user.as_deref());
    let style = if std::env::var_os("NO_COLOR").is_some() {
        env_logger::WriteStyle::Never
    } else {
        env_logger::WriteStyle::Always
    };
    let _ = env_logger::Builder::new()
        .parse_filters(&filter)
        .write_style(style)
        .try_init();
}

fn resolved_log_filter(user: Option<&str>) -> String {
    match user {
        Some(s) if !s.is_empty() => format!("info,{s}"),
        _ => "info".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::resolved_log_filter;

    /// Whether `target` logs at `level` under the filter `user` asked for.
    ///
    /// Builds the logger without installing it, which is what lets the
    /// sub-target rules below be asserted rather than described: the mods
    /// split one gauge's output across a parent target and its children
    /// precisely so a reader can silence a cadence, and that only works if
    /// the longest matching directive is the one that decides.
    fn enabled(user: &str, target: &str, level: log::Level) -> bool {
        use log::Log as _;

        env_logger::Builder::new()
            .parse_filters(&resolved_log_filter(Some(user)))
            .build()
            .enabled(&log::Metadata::builder().level(level).target(target).build())
    }

    #[test]
    fn a_child_target_can_be_silenced_without_silencing_its_parent() {
        let f = "wow=debug,wow::gauge::live=info";
        assert!(enabled(f, "wow::gauge", log::Level::Debug));
        assert!(enabled(f, "wow::gauge::script", log::Level::Debug));
        assert!(!enabled(f, "wow::gauge::live", log::Level::Debug));
        // Still a warning channel: silencing is a volume control, not a mute.
        assert!(enabled(f, "wow::gauge::live", log::Level::Warn));
    }

    #[test]
    fn arming_a_parent_arms_every_child() {
        let f = "wow=debug";
        for t in [
            "wow::gauge",
            "wow::gauge::live",
            "wow::gauge::script",
            "wow::gc",
        ] {
            assert!(enabled(f, t, log::Level::Debug), "{t}");
        }
    }

    #[test]
    fn an_unasked_for_namespace_stays_at_the_info_baseline() {
        let f = "wow::gauge=debug";
        assert!(!enabled(f, "wow::gc", log::Level::Debug));
        assert!(enabled(f, "wow::gc", log::Level::Info));
    }

    #[test]
    fn unset_defaults_to_info() {
        assert_eq!(resolved_log_filter(None), "info");
    }

    #[test]
    fn empty_string_defaults_to_info() {
        assert_eq!(resolved_log_filter(Some("")), "info");
    }

    #[test]
    fn bare_level_wins_via_last_spec() {
        assert_eq!(resolved_log_filter(Some("warn")), "info,warn");
    }

    #[test]
    fn root_spec_composes() {
        assert_eq!(resolved_log_filter(Some("wow=warn")), "info,wow=warn");
    }

    #[test]
    fn sub_namespace_override_restores_baseline() {
        assert_eq!(
            resolved_log_filter(Some("wow::perf=debug")),
            "info,wow::perf=debug"
        );
    }

    #[test]
    fn multi_spec_passthrough() {
        assert_eq!(
            resolved_log_filter(Some("wow=warn,wow::dxso=trace")),
            "info,wow=warn,wow::dxso=trace",
        );
    }
}
