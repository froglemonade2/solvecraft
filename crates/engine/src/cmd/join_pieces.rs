//! What a feature's material does when it is joined: the test the feature windows use to choose
//! Join, New Body or Cut by themselves (#6901). It runs the Join itself, so it agrees with the
//! result OK makes, and with the warning a Join apart gets (#4434).

use serde_json::Value;

use crate::Session;

/// The most commands a feature window previews at once.
const MAX_COMMANDS: usize = 64;

/// What a feature's material makes, joined to the bodies it may join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Addition {
    /// It would be a separate piece (or there is no body to join): a New Body.
    Apart,
    /// It makes one body with what it touches: a Join.
    Joins,
    /// It adds no material: it lies inside a body (a Cut, where the window offers one).
    Inside,
}

/// What `cmds` (a feature window's commands, with operation New Body or Join) make when run with
/// Join. None when they have no such operation (Combine, a Cut chosen by hand, a feature without
/// one), the model has no body, or the Join fails.
pub fn addition(s: &Session, cmds: &[(String, Value)]) -> Option<Addition> {
    if cmds.is_empty() || cmds.len() > MAX_COMMANDS || s.model.state().bodies.is_empty() {
        return None;
    }
    let mut joined = cmds.to_vec();
    let mut any = false;
    for (command, p) in &mut joined {
        let inner = p.get("command").and_then(Value::as_str).unwrap_or(command.as_str());
        if inner == "solid.combine" {
            return None;
        }
        let fields = if p.get("command").is_some() { p.get_mut("params")? } else { p };
        if let Some(op) = fields.get_mut("operation")
            && op.as_str().is_some_and(|o| o == "new" || o == "join")
        {
            *op = Value::from("join");
            any = true;
        }
    }
    if !any {
        return None;
    }
    let p = s.preview(&joined).ok()?;
    // A Join with nothing to join makes a new body.
    if p.after.bodies.iter().any(|b| p.before.body(&b.name).is_none()) || !solidcraft_doc::join_pieces_added(&p.before, &p.after).is_empty() {
        return Some(Addition::Apart);
    }
    let volume = |b: &solidcraft_doc::ModelBody| solidcraft_doc::kernel::measure(&b.body).ok().map(|m| m.volume).filter(|v| v.is_finite());
    // The volume of the bodies the Join changed or absorbed, before and after.
    let (mut had, mut have) = (0.0, 0.0);
    for was in &p.before.bodies {
        match p.after.body(&was.name) {
            Some(now) if now.body.same_shape(&was.body) => {}
            Some(now) => {
                had += volume(was)?;
                have += volume(now)?;
            }
            None => had += volume(was)?,
        }
    }
    let added = have - had;
    Some(if added <= had.abs() * 1e-6 + 1e-9 { Addition::Inside } else { Addition::Joins })
}

#[cfg(test)]
#[path = "join_pieces_tests.rs"]
mod tests;
