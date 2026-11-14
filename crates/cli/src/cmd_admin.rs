//! `tools`, `settings`, `cache`, `schedule`, `run-profile`, `context-menu`, `info`,
//! `commands`. Owner: W3-E.

use mj_core::{Cadence, Schedule, Settings};
use mj_engine::api::{IdRequest, ScheduleFromPlanRequest, SettingsSetRequest};
use mj_engine::paths::integration::{context_menu, scheduler, StdRunner};
use serde_json::Value;

use crate::args::Parsed;
use crate::cmd_plan::finish;
use crate::ctx::{Ctx, Fail, Res};
use crate::out::{bytes, json, table, when};

/// `info`.
pub fn info(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    println!("{}", json(&ctx.engine.app_info()?));
    Ok(())
}

/// `commands`.
pub fn commands(_: &Parsed) -> Res {
    for c in mj_engine::api::COMMANDS {
        println!("{}", c.name);
    }
    Ok(())
}

/// `tools [--refresh]`.
pub fn tools(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let list = if a.flag("refresh") { ctx.engine.tools_refresh()? } else { ctx.engine.tools_list()? };
    if ctx.json {
        println!("{}", json(&list));
        return Ok(());
    }
    let mut rows = vec![["TOOL", "STATUS", "VERSION", "WHERE / WHY", "USED FOR"].map(String::from).to_vec()];
    for t in &list {
        let (status, place) = if t.found {
            ("found", t.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default())
        } else {
            ("missing", format!("not found; install it or set `settings set tool_paths.{} <ABSOLUTE-PATH>`", t.id))
        };
        rows.push(vec![
            t.id.clone(),
            status.into(),
            t.version.clone().unwrap_or_default(),
            place,
            t.used_for.join(", "),
        ]);
    }
    print!("{}", table(&rows, &[]));
    Ok(())
}

fn lookup<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.').try_fold(v, |cur, k| cur.get(k))
}

fn show_value(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    }
}

/// Set `key` (dotted) to `value` inside a JSON tree; the parents must exist.
fn set_path(root: &mut Value, key: &str, value: Value) -> Result<(), String> {
    let parts: Vec<&str> = key.split('.').collect();
    let (last, parents) = parts.split_last().ok_or("empty key")?;
    let mut cur = root;
    for p in parents {
        cur = cur.get_mut(*p).ok_or_else(|| format!("unknown setting `{key}`: no `{p}` section"))?;
    }
    match cur {
        Value::Object(m) => {
            m.insert((*last).to_string(), value);
            Ok(())
        }
        _ => Err(format!("unknown setting `{key}`")),
    }
}

/// Parse a command-line value: JSON when it is JSON, otherwise a plain string.
fn parse_value(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.to_string()))
}

/// `settings get|set`.
pub fn settings(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let current = ctx.engine.settings_get()?;
    let mut tree = serde_json::to_value(&current).map_err(|e| Fail::Error(e.to_string()))?;
    if a.cmd.sub == Some("get") {
        match a.positionals.first() {
            None => println!("{}", json(&tree)),
            Some(k) => println!(
                "{}",
                show_value(lookup(&tree, k).ok_or_else(|| Fail::Error(format!(
                    "unknown setting `{k}`; run `settings get` to see them all"
                )))?)
            ),
        }
        return Ok(());
    }
    let key = &a.positionals[0];
    let value = parse_value(&a.positionals[1]);
    set_path(&mut tree, key, value.clone()).map_err(Fail::Error)?;
    let next: Settings =
        serde_json::from_value(tree).map_err(|e| Fail::Error(format!("`{key}` does not accept {value}: {e}")))?;
    let saved = ctx.engine.settings_set(SettingsSetRequest { settings: next })?;
    let after = serde_json::to_value(&saved).map_err(|e| Fail::Error(e.to_string()))?;
    if lookup(&after, key).is_none() {
        return Err(Fail::Error(format!("unknown setting `{key}`: nothing was changed")));
    }
    println!("{key} = {}", show_value(lookup(&after, key).unwrap_or(&Value::Null)));
    Ok(())
}

/// `cache info|clear`.
pub fn cache(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let info = if a.cmd.sub == Some("clear") { ctx.engine.cache_clear()? } else { ctx.engine.cache_info()? };
    if ctx.json {
        println!("{}", json(&info));
    } else {
        let verb = if a.cmd.sub == Some("clear") { "Cleared. " } else { "" };
        println!(
            "{verb}Scan cache: {} entries, {}{}",
            info.entries,
            bytes(info.bytes),
            info.path.map(|p| format!(" ({})", p.display())).unwrap_or_default()
        );
    }
    Ok(())
}

fn find_schedule(ctx: &Ctx, name_or_id: &str) -> Res<Schedule> {
    let all = ctx.engine.schedules_list()?;
    let lower = name_or_id.to_lowercase();
    all.into_iter()
        .find(|s| s.id == name_or_id || s.name.to_lowercase() == lower)
        .ok_or_else(|| Fail::Error(format!("no schedule named `{name_or_id}`; see `schedule list`")))
}

fn scheduler_ctx(ctx: &Ctx) -> Res<scheduler::Ctx> {
    let cli_exe = std::env::current_exe().map_err(|e| Fail::Error(e.to_string()))?;
    let default = mj_engine::paths::detect();
    Ok(scheduler::Ctx { cli_exe, data_dir: (ctx.dirs.data != default.data).then(|| ctx.dirs.data.clone()) })
}

fn cadence(a: &Parsed) -> Res<Cadence> {
    let at = a.value("at").unwrap_or("08:47");
    let (h, m) = at
        .split_once(':')
        .and_then(|(h, m)| Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?)))
        .ok_or_else(|| Fail::Usage(format!("--at must look like 08:47, got `{at}`")))?;
    let num = |name: &str, default: u8| -> Res<u8> { Ok(a.number(name)?.map(|n| n as u8).unwrap_or(default)) };
    Ok(match a.value("every").unwrap_or("weekly") {
        "daily" => Cadence::Daily { hour: h, minute: m },
        "weekly" => Cadence::Weekly { weekday: num("weekday", 1)?, hour: h, minute: m },
        "monthly" => Cadence::Monthly { day: num("day", 1)?, hour: h, minute: m },
        other => return Err(Fail::Usage(format!("--every must be daily, weekly or monthly, got `{other}`"))),
    })
}

/// `schedule list|from-plan|install|uninstall|delete`.
pub fn schedule(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    match a.cmd.sub {
        Some("from-plan") => {
            let c = cadence(a)?;
            let s = ctx.ensure_scanned()?;
            ctx.replay_plan(&s.plan, None)?;
            let saved = ctx
                .engine
                .schedule_from_plan(ScheduleFromPlanRequest { name: a.positionals[0].clone(), cadence: c })?;
            println!(
                "Saved schedule {} ({}). Register it with the OS using `schedule install {}`.",
                saved.id, saved.name, saved.id
            );
        }
        Some("install") => {
            let s = find_schedule(&ctx, &a.positionals[0])?;
            let next = scheduler::install(&s, &scheduler_ctx(&ctx)?, &StdRunner)?;
            println!("Registered `{}` ({}).", s.name, s.id);
            for c in next {
                println!("  run: {c}");
            }
        }
        Some("uninstall") => {
            let s = find_schedule(&ctx, &a.positionals[0])?;
            for c in scheduler::uninstall(&s.id, &StdRunner)? {
                println!("  run: {c}");
            }
            println!("Removed the OS registration of `{}`.", s.name);
        }
        Some("delete") => {
            let s = find_schedule(&ctx, &a.positionals[0])?;
            let _ = scheduler::uninstall(&s.id, &StdRunner);
            ctx.engine.schedule_delete(IdRequest { id: s.id.clone() })?;
            println!("Deleted `{}`.", s.name);
        }
        _ => {
            let list = ctx.engine.schedules_list()?;
            if ctx.json {
                println!("{}", json(&list));
            } else if list.is_empty() {
                println!("No schedules. Save the plan as one with `schedule from-plan <NAME>`.");
            } else {
                let mut rows =
                    vec![["ID", "NAME", "ON", "NEXT RUN", "LAST RUN", "STEPS", "OS"].map(String::from).to_vec()];
                for s in &list {
                    let os = if scheduler::is_installed(&s.id, &StdRunner) { "registered" } else { "not registered" };
                    rows.push(vec![
                        s.id.clone(),
                        s.name.clone(),
                        if s.enabled { "yes" } else { "no" }.into(),
                        s.next_run.map(when).unwrap_or_else(|| "-".into()),
                        s.last_run.map(when).unwrap_or_else(|| "never".into()),
                        s.steps.len().to_string(),
                        os.into(),
                    ]);
                }
                print!("{}", table(&rows, &[5]));
            }
        }
    }
    Ok(())
}

/// `run-profile <name>`.
pub fn run_profile(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let s = find_schedule(&ctx, &a.positionals[0])?;
    let started = match ctx.engine.schedule_run_now(IdRequest { id: s.id.clone() }) {
        Ok(st) => st,
        Err(mj_core::Error::InvalidInput(m)) if m.starts_with("nothing to do") => {
            println!("{}: {m}", s.name);
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let jobs = ctx.wait_apply(&started)?;
    finish(&ctx, started.run_id.0, &jobs)
}

/// `context-menu status|install|uninstall`.
pub fn context_menu_cmd(a: &Parsed) -> Res {
    let ctx = Ctx::open(a)?;
    let env = mj_engine::paths::current_env();
    match a.cmd.sub {
        Some("install") | Some("uninstall") => {
            if mj_engine::paths::integration::os_integration_disabled() {
                return Err(Fail::Error(format!(
                    "OS integration is disabled by {}",
                    mj_engine::paths::integration::NO_OS_INTEGRATION_VAR
                )));
            }
            let on = a.cmd.sub == Some("install");
            let mut s = ctx.engine.settings_get()?;
            if s.integration.context_menu == on {
                // Already in the requested state in the settings: redo the OS part.
                if on {
                    context_menu::install(&StdRunner)?;
                } else {
                    context_menu::remove(&StdRunner)?;
                }
            } else {
                s.integration.context_menu = on;
                ctx.engine.settings_set(SettingsSetRequest { settings: s })?;
            }
            println!("Context menu {}.", if on { "installed" } else { "removed" });
        }
        _ => {
            let st = context_menu::query_status(mj_engine::paths::Os::current(), &env, &StdRunner);
            println!(
                "Context menu: {}{}",
                if !st.available {
                    "not available"
                } else if st.installed {
                    "installed"
                } else {
                    "not installed"
                },
                st.reason.map(|r| format!(" ({r})")).unwrap_or_default()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_paths_read_and_write() {
        let mut v = serde_json::to_value(Settings::default()).unwrap();
        assert_eq!(lookup(&v, "performance.threads"), Some(&Value::from(0)));
        assert_eq!(lookup(&v, "performance.nope"), None);
        set_path(&mut v, "performance.threads", Value::from(4)).unwrap();
        assert_eq!(lookup(&v, "performance.threads"), Some(&Value::from(4)));
        assert!(set_path(&mut v, "nosuch.x", Value::Null).is_err());
        assert!(set_path(&mut v, "performance.threads.deep", Value::Null).is_err());
    }

    #[test]
    fn values_parse_as_json_or_text() {
        assert_eq!(parse_value("4"), Value::from(4));
        assert_eq!(parse_value("true"), Value::Bool(true));
        assert_eq!(parse_value("[\"a\"]"), serde_json::json!(["a"]));
        assert_eq!(parse_value("/tmp/x"), Value::String("/tmp/x".into()));
    }
}
