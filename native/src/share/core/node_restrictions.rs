use super::ShareIrohNode;
use crate::share::{
    core::eio,
    relation_rights::{RestrictionReason, RestrictionSet},
    session::PeerPrincipal,
};
use iroh::endpoint::{Connection, VarInt};
use std::{collections::HashSet, io, sync::atomic::Ordering};

impl ShareIrohNode {
    pub(super) async fn bind_incoming_principal(
        &self,
        connection: &Connection,
        principal: PeerPrincipal,
    ) -> io::Result<()> {
        let generation = self.policy.snapshot(principal.clone())?;
        let ticket = self.policy.connection_ticket(&principal)?;
        let permit = if let Some(permit) = ticket.try_acquire()? {
            permit
        } else {
            self.request_connection_yield(&principal)?;
            tokio::select! {
                permit = ticket.acquire() => permit?,
                _ = connection.closed() => return Err(eio("Share-Anmeldung beendet")),
            }
        };
        self.require_sharing_active()?;
        self.policy
            .bind_admitted(connection, generation, Some(permit))
    }
    fn request_connection_yield(&self, principal: &PeerPrincipal) -> io::Result<()> {
        let incoming = self
            .incoming_sessions
            .lock()
            .map_err(|_| eio("Share-Sitzungen gesperrt"))?;
        let candidates: Vec<_> = incoming
            .values()
            .filter(|entry| {
                entry.activity.as_ref().is_some_and(|activity| {
                    !activity.fair_yield_requested() && activity.can_yield_connection()
                })
            })
            .map(|entry| entry.connection.stable_id())
            .collect();
        if let Some(id) = self.policy.borrowed_connection(&candidates, principal)? {
            if let Some(activity) = incoming
                .values()
                .find(|entry| entry.connection.stable_id() == id)
                .and_then(|entry| entry.activity.as_ref())
            {
                activity.request_fair_yield();
            }
        }
        Ok(())
    }

    pub(super) fn invalidate_sessions(&self) -> io::Result<usize> {
        self.invalidate_restrictions(&RestrictionSet::everything(RestrictionReason::Unattributed))
    }

    pub(crate) fn invalidate_restrictions(
        &self,
        restrictions: &RestrictionSet,
    ) -> io::Result<usize> {
        let epoch = self
            .auth
            .lock()
            .map_err(|_| eio("Share-State gesperrt"))?
            .authorization_epoch;
        self.invalidate_restrictions_at(restrictions, epoch)
    }

    pub(super) fn invalidate_restrictions_at(
        &self,
        restrictions: &RestrictionSet,
        epoch: u64,
    ) -> io::Result<usize> {
        if restrictions.is_empty() {
            return Ok(0);
        }
        let previous = self
            .session_epoch
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |epoch| {
                epoch.checked_add(1)
            })
            .map_err(|_| eio("Share session epoch exhausted"))?;
        let mut connections = self.policy.invalidate(restrictions, previous + 1)?;
        let mut ids: HashSet<_> = connections.iter().map(Connection::stable_id).collect();
        // An unbound handshake has no application authority yet. Close that
        // unknown transport conservatively, without disturbing bound peers.
        let mut incoming = self
            .incoming_sessions
            .lock()
            .map_err(|_| eio("Eingehende Share-Sessions gesperrt"))?;
        for entry in incoming.values() {
            if !self.policy.is_bound(&entry.connection)? && ids.insert(entry.connection.stable_id())
            {
                connections.push(entry.connection.clone());
            }
        }
        incoming.retain(|_, entry| !ids.contains(&entry.connection.stable_id()));
        drop(incoming);
        let mut outgoing = self
            .sessions
            .lock()
            .map_err(|_| eio("Ausgehende Share-Sessions gesperrt"))?;
        for connection in outgoing.values() {
            if !self.policy.is_bound(connection)? && ids.insert(connection.stable_id()) {
                connections.push(connection.clone());
            }
        }
        outgoing.retain(|_, connection| !ids.contains(&connection.stable_id()));
        drop(outgoing);
        let leases = self.mount_leases.invalidate(restrictions);
        let exec = self
            .exec_registry()
            .restrict_authorization(epoch, restrictions)
            .map_err(eio);
        for connection in connections {
            connection.close(VarInt::from_u32(0x5345), b"authorization changed");
        }
        let leases = leases?;
        if !leases.is_empty() {
            self.rt.spawn(async move {
                let _ = crate::share::blocking::run("Share dispose revoked leases", move || {
                    drop(leases);
                    Ok(())
                })
                .await;
            });
        }
        exec?;
        Ok(ids.len())
    }
}
