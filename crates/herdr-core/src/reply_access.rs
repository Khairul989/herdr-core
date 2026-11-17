//! Explicit message-only return routes for independently started remote agents.

use crate::rpc::RpcError;
use crate::runtime::State;
use crate::terminals::{Terminal, parse_id};

pub(crate) fn live_sender<'a>(st: &'a State, caller: Option<&str>) -> Result<&'a Terminal, RpcError> {
    caller.and_then(parse_id).and_then(|id| st.reg.terminals.get(&id)).filter(|t| t.is_live()).ok_or_else(|| {
        RpcError::new(
            "stale_identity",
            "the sender is not a live terminal on this core",
            "run from your own live hc terminal; start Codex with --no-daemon to keep its terminal identity",
        )
    })
}

pub(crate) fn check_sender(st: &State, caller: Option<&str>) -> Result<(), RpcError> {
    // Plain-shell callers and already-qualified remote callers have no local terminal id.
    if caller.is_some_and(|c| parse_id(c).is_some()) {
        live_sender(st, caller)?;
    }
    Ok(())
}

pub(crate) fn permits(t: &Terminal, machine: &str, method: &str, caller: Option<&str>) -> bool {
    method == "terminal.send"
        && t.is_live()
        && caller
            .and_then(parse_id)
            .is_some_and(|id| t.reply_from.contains(&format!("{machine}/{}", crate::terminals::address(id))))
}

pub(crate) async fn set(
    core: &crate::runtime::Core,
    p: crate::api::ReplyAccessParams,
) -> Result<crate::api::ReplyAccessResult, RpcError> {
    let result = {
        let mut st = core.lock();
        let local = live_sender(&st, p.caller.as_deref())?.address.clone();
        let (machine, remote) = p
            .terminal
            .split_once('/')
            .ok_or_else(|| RpcError::invalid_params("reply access needs a remote address like pve/t3"))?;
        let remote_id = parse_id(remote)
            .ok_or_else(|| RpcError::invalid_params("reply access needs a remote terminal id like pve/t3"))?;
        let m = st
            .machines
            .iter()
            .find(|m| m.name == machine || m.id == machine)
            .ok_or_else(|| crate::machines::unknown(&st.machines, machine))?;
        let address = format!("{}/{}", m.name, crate::terminals::address(remote_id));
        let key = format!("{}/{}", m.id, crate::terminals::address(remote_id));
        if p.allow
            && !st
                .mirrors
                .get(&m.id)
                .and_then(|terminals| terminals.get(&address))
                .is_some_and(|t| t["state"] == "live")
        {
            return Err(RpcError::new("not_found", format!("no live remote terminal {address}"), "see hc ls"));
        }
        let sender = st.reg.terminals.get_mut(&parse_id(&local).expect("live local id")).expect("live sender");
        if p.allow {
            sender.reply_from.insert(key);
        } else {
            sender.reply_from.remove(&key);
        }
        core.save(&st);
        crate::api::ReplyAccessResult { terminal: local, remote: address, allowed: p.allow }
    };
    core.flush_saved().await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminals::{NewTerminal, Public};

    #[test]
    fn grants_are_private_persistent_and_only_authorize_the_selected_live_pair() {
        let mut st = State::default();
        let id = st.reg.create(NewTerminal { argv: vec!["cat".into()], ..Default::default() }, 0).unwrap();
        let t = st.reg.terminals.get_mut(&id).unwrap();
        assert!(!permits(t, "remote-core", "terminal.send", Some("t3")));
        t.reply_from.insert("remote-core/t3".into());
        assert!(permits(t, "remote-core", "terminal.send", Some("t3")));
        for method in ["terminal.read", "terminal.get", "terminal.wait", "terminal.keys", "notes"] {
            assert!(!permits(t, "remote-core", method, Some("t3")));
        }
        assert!(!permits(t, "other-core", "terminal.send", Some("t3")));
        assert!(!permits(t, "remote-core", "terminal.send", Some("t4")));
        assert!(!permits(t, "remote-core", "terminal.send", None));
        let saved = serde_json::to_value(&*t).unwrap();
        let restored: Terminal = serde_json::from_value(saved.clone()).unwrap();
        assert!(permits(&restored, "remote-core", "terminal.send", Some("t3")));
        assert!(serde_json::to_value(Public(restored)).unwrap().get("reply_from").is_none());
        let mut old = saved;
        old.as_object_mut().unwrap().remove("reply_from");
        let old: Terminal = serde_json::from_value(old).unwrap();
        assert!(old.reply_from.is_empty());
        t.state = "finished".into();
        assert!(!permits(t, "remote-core", "terminal.send", Some("t3")));
        assert!(live_sender(&st, Some("t1")).is_err());
        assert!(live_sender(&st, Some("t99999")).is_err());
    }
}
