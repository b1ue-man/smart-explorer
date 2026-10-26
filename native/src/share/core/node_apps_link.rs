//! Pinned outbound device commands. Trusted RDP inventories arise here only.
use super::{
    apps_wire::{self, AppsClientHello, AppsCommand, AppsReply, AppsResult, AppsServerHello},
    core::{eio, hmac_proof, random_token},
    service::ShareService,
    session::{endpoint_addr, relation_kind_id, PeerPrincipal},
    types::{PeerEndpoint, PeerOpenTarget},
};
use iroh::endpoint::Connection;
use std::{io, sync::Arc, time::Duration};

/// This receipt is created only after the transport/identity/request checks.
/// It deliberately does not implement Deserialize.
pub(crate) struct AuthenticatedAppsReply {
    pub node_id: String,
    pub device_id: String,
    pub result: AppsResult,
}
impl ShareService {
    pub(crate) fn apps_command(
        &self,
        target: &PeerOpenTarget,
        command: AppsCommand,
        mut pending: impl FnMut(),
        cancellation: Option<Arc<super::route_cancellation::RouteCancellation>>,
    ) -> io::Result<AuthenticatedAppsReply> {
        let endpoint = self.endpoint_for_target(target).map_err(eio)?;
        let node = self.iroh.clone();
        let identity = self.identity.clone();
        node.block_on(async {
            let cancelled=async {if let Some(cancel)=&cancellation {loop {
                cancel.check()?;tokio::select! {()=cancel.cancelled()=>return Err::<(),io::Error>(eio("device command cancelled")),
                    _=tokio::time::sleep(Duration::from_millis(100))=>{}}
            }} else {std::future::pending::<io::Result<()>>().await}};
            tokio::select! {biased;
                result=cancelled=>{result?;Err(eio("device command cancelled"))},
                result=async {
                    let _slot=node.apps.slots.clone().try_acquire_owned().map_err(|_|eio("device command admission exhausted"))?;
                    node.require_sharing_active()?;
                    if endpoint.expected_node_id.as_deref().is_some_and(|pin|pin.is_empty()||pin!=endpoint.presence.node_id) {
                        return Err(eio("device command NodeId pin mismatch"));
                    }
                    let local=node.routes.current(&node.endpoint);
                    let address=endpoint_addr(&endpoint.presence,&local,&node.transport_options)?;
                    let generation=node.policy.snapshot(PeerPrincipal::from_endpoint(&endpoint))?;
                    let connection=tokio::time::timeout(Duration::from_secs(30),node.endpoint.connect(address,apps_wire::APPS_ALPN)).await
                        .map_err(|_|eio("device command connect timed out"))?.map_err(eio)?;
                    let _close=super::node_tcp::CloseTcp(connection.clone());
                    super::node_tcp::require_policy(&connection)?;node.policy.bind(&connection,generation)?;
                    if let Some(cancel)=&cancellation {cancel.bind(&connection)?;}
                    node.await_route(&connection,Some(&endpoint),None).await?;
                    super::route_protocol::scope(connection.clone(),async {
                        let deadline=super::route_protocol::active_now()+Duration::from_secs(20);
                        let (mut send,mut recv)=super::route_protocol::run_until_active(deadline,"device command hello timeout",async {connection.accept_bi().await.map_err(eio)}).await?;
                        let server:AppsServerHello=super::route_protocol::run_until_active(deadline,"device command server hello timeout",apps_wire::receive_hello(&mut recv)).await?;
                        check_server(&server,&endpoint,&connection)?;
                        super::route_monitor::authenticated_remote_capability(&connection,server.challenge.starts_with("rp1."));
                        let (kind,relation)=relation_kind_id(&endpoint);
                        let mut client=AppsClientHello {version:apps_wire::APPS_VERSION,relation_kind:kind.into(),relation_id:relation,
                            device_id:identity.device_id.clone(),device_name:identity.device_name.clone(),public_key:identity.public_key.clone(),
                            fingerprint:identity.fingerprint.clone(),node_id:identity.node_id.clone(),nonce:format!("rp1.{}",random_token(24).map_err(eio)?),
                            proof:String::new(),id:random_token(24).map_err(eio)?,command:command.clone()};
                        client.validate()?;
                        client.proof=hmac_proof(&endpoint.relation_secret,&super::relation_proof::relation_transcript(&server.relation(&client)?,&client.relation()));
                        super::route_protocol::run_until_active(deadline,"device command client hello timeout",apps_wire::send(&mut send,&client)).await?;
                        let mut reply:AppsReply=super::route_protocol::timeout_for(Duration::from_secs(30),apps_wire::receive(&mut recv)).await??;
                        let mut asked=false;
                        loop {match reply {
                            AppsReply::Pending if !asked=>{asked=true;pending();},
                            AppsReply::Pending=>return Err(eio("duplicate device command pending reply")),
                            AppsReply::Refused {detail}=>return Err(io::Error::new(io::ErrorKind::PermissionDenied,detail)),
                            AppsReply::Completed {id,request_hash,result}=>{
                                if id!=client.id||request_hash!=client.request_hash()?||!matches_result(&client.command,&result) {return Err(eio("device command response binding mismatch"));}
                                if let AppsResult::Catalog {catalog}=&result {if catalog.device_id.as_str()!=server.device_id {return Err(eio("catalog target device mismatch"));}}
                                if let (AppsCommand::Icon {app_id,icon,..},AppsResult::Icon {reply:Some(reply)})=(&client.command,&result) {
                                    if app_id.as_str()!=icon.app_id {return Err(eio("icon app identity mismatch"));}reply.bytes_for(icon)?;
                                }
                                if let AppsResult::Launch {receipt}|AppsResult::Receipt {receipt:Some(receipt)}=&result {
                                    let request=match &client.command {AppsCommand::Launch {request}|AppsCommand::Receipt {request}=>request,_=>return Err(eio("unexpected launch receipt"))};
                                    if &receipt.request!=request {return Err(eio("launch receipt request mismatch"));}
                                }
                                return Ok(AuthenticatedAppsReply {node_id:server.node_id,device_id:server.device_id,result});
                            }
                        }
                        reply=apps_wire::receive(&mut recv).await?;
                        }
                    }).await
                }=>result,
            }
        })
    }
    pub(crate) fn rdp_host_probe(
        &self,
        target: &PeerOpenTarget,
        pending: impl FnMut(),
        cancellation: Option<Arc<super::route_cancellation::RouteCancellation>>,
    ) -> io::Result<crate::rdp::AuthenticatedProbe> {
        let reply = self.apps_command(target, AppsCommand::RdpProbe, pending, cancellation)?;
        let AppsResult::RdpProbe { host } = reply.result else {
            return Err(eio("unexpected RDP host probe reply"));
        };
        crate::rdp::AuthenticatedProbe::from_authenticated_peer(reply.node_id, host).map_err(eio)
    }
}
fn check_server(
    server: &AppsServerHello,
    endpoint: &PeerEndpoint,
    connection: &Connection,
) -> io::Result<()> {
    let p = &endpoint.presence;
    if server.version != apps_wire::APPS_VERSION
        || server.device_id != p.device_id
        || server.public_key != p.public_key
        || server.fingerprint != p.fingerprint
        || server.node_id != p.node_id
        || server.node_id != connection.remote_id().to_string()
        || !super::profiles::fingerprint_matches(&server.public_key, &server.fingerprint)
        || server.challenge.len() < 24
        || server.challenge.len() > 256
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "device command server does not match pinned peer",
        ));
    }
    Ok(())
}
fn matches_result(command: &AppsCommand, result: &AppsResult) -> bool {
    matches!(
        (command, result),
        (AppsCommand::Catalog, AppsResult::Catalog { .. })
            | (AppsCommand::Launch { .. }, AppsResult::Launch { .. })
            | (AppsCommand::Icon { .. }, AppsResult::Icon { .. })
            | (AppsCommand::Receipt { .. }, AppsResult::Receipt { .. })
            | (AppsCommand::RdpProbe, AppsResult::RdpProbe { .. })
    )
}
