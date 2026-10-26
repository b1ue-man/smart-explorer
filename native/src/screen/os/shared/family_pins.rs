//! Verified listing pins, launch receipts and one-time worker transfer.
use super::*;
pub(in crate::screen::os::host_worker_client) struct PinnedFamily {
    adapter: std::sync::Arc<std::sync::Mutex<Box<dyn WindowAdapter>>>,
    binding: WindowBinding,
    lifetime: String,
}
impl PinnedFamily {
    pub(in crate::screen::os::host_worker_client) fn prepare(self,program:&std::path::Path,env:&[(String,String)],owner:&str,session:&str,source:&SourceId,
        wanted:Option<crate::screen::core::pixel_convert::FrameSize>,control:bool,
        admission:std::sync::Arc<crate::screen::core::capture_admission::SessionAdmission>)
        ->Result<super::super::prepared::PreparedWorker,crate::screen::core::host_backend::SourceRefusal> {
        let refusal=|error:io::Error|crate::screen::core::host_backend::SourceRefusal::new(&error.to_string());
        let pin=self.adapter.lock().map_err(|_|crate::screen::core::host_backend::SourceRefusal::new("family pin store poisoned"))?
            .export_pin(&self.binding).map_err(refusal)?;
        let prepared=super::super::prepared::PreparedWorker::new(program,env,owner,session,source,wanted,control,admission,
            Some(crate::screen::core::worker_family::OpenFamily{pin,lifetime:self.lifetime.clone(),session:session.into(),control,wanted}))?;
        self.adapter.lock().map_err(|_|crate::screen::core::host_backend::SourceRefusal::new("family pin store poisoned"))?
            .transfer_pin(&self.binding);Ok(prepared)
    }
    pub(in crate::screen::os::host_worker_client) fn open(self, program: &std::path::Path, env: &[(String,String)],
        session: &str, control: bool, wanted: Option<crate::screen::core::pixel_convert::FrameSize>)
        -> Result<super::super::WorkerSource, crate::screen::core::host_backend::SourceRefusal> {
        let pin = self.adapter.lock().map_err(|_|crate::screen::core::host_backend::SourceRefusal::new("family_pin_store_poisoned"))?
            .export_pin(&self.binding)
            .map_err(|error| crate::screen::core::host_backend::SourceRefusal::new(&error.to_string()))?;
        let opened = super::super::WorkerSource::open_family(program, env,
            crate::screen::core::worker_family::OpenFamily { pin, lifetime: self.lifetime.clone(),
                session: session.into(), control, wanted })?;
        // The receiver has validated the original marker and opened the family.
        // Only now may its original owner relinquish cleanup responsibility.
        self.adapter.lock().map_err(|_|crate::screen::core::host_backend::SourceRefusal::new("family_pin_store_poisoned"))?
            .transfer_pin(&self.binding);
        Ok(opened)
    }
}

#[derive(Default)]
pub(in crate::screen::os::host_worker_client) struct FamilyPins {
    owners: HashMap<String, HashMap<String, PinnedFamily>>,
    registered: HashMap<String,HashMap<String,(FamilyListEntry,PinnedFamily,crate::remote_window::WindowPin)>>,
    receipts:HashMap<String,HashMap<String,(FamilyListEntry,crate::remote_window::WindowPin)>>,
}
impl FamilyPins {
    pub(in crate::screen::os::host_worker_client) fn list(&mut self, owner: &str, raw: WindowList) -> io::Result<FamilyListing> {
        // One X11 connection/namespace per listing, not one server client per
        // window. Its pins stay alive through shared ownership until transfer.
        let adapter=std::sync::Arc::new(std::sync::Mutex::new(crate::remote_window::os::adapter()?));
        if let Some(registered)=self.registered.get_mut(owner) {
            registered.retain(|_,(_,pin,_)|pin.adapter.lock().ok().is_some_and(|mut adapter|adapter.validate(&pin.binding).is_ok()));
        }
        let raw = raw.bounded();
        let mut pinned = Vec::new();
        let mut limitations = raw.limits;
        for mut window in raw.windows {
            let Some(native) = window.id.parse::<u64>().ok().filter(|id| *id != 0) else { continue; };
            let result = (|| {
                let mut platform=adapter.lock().map_err(|_|io::Error::other("family_pin_store_poisoned"))?;
                let binding = platform.bind(NativeWindowId(native))?;
                let root = platform.inspect(&binding)?;
                // Preserve P3's catalog mapping only when the freshly pinned
                // platform app label still matches. It is display metadata;
                // the independent OS marker is the selection proof.
                let app_id=root.app_id.clone().or_else(||(window.app==root.app).then(||window.app_id.clone()).flatten());
                let icon = window.icon.take().filter(|icon| app_id.as_deref() == Some(icon.app_id.as_str()));
                window.title = root.title.clone(); window.app = root.app.clone(); window.app_id = app_id;
                window.icon = icon; window.pid = None; window.focused = root.focused; window.minimized = root.minimized;
                platform.validate(&binding)?;
                let lifetime = token()?;
                let entry = FamilyListEntry { launch_request:None,source: SourceId::Window(format!("{FAMILY_PREFIX}{lifetime}")),
                    identity: WindowIdentity { session: lifetime.clone(), generation: 1 }, window: window.clone() };
                Ok::<_,io::Error>((entry, PinnedFamily { adapter:adapter.clone(), binding, lifetime }))
            })();
            match result {
                Ok(entry) => pinned.push(entry),
                Err(error) => limitations.push(format!("family_pin_unavailable:{native}:{error}")),
            }
        }
        if let Some(registered)=self.registered.get(owner) {
            pinned.retain(|(_,pin)|!registered.values().any(|(_,known,_)|known.binding.native==pin.binding.native));
        }
        loop {
            let mut entries:Vec<_>=self.registered.get(owner).into_iter().flat_map(|pins|pins.values().map(|(entry,_,_)|entry.clone())).collect();
            entries.extend(pinned.iter().map(|(entry,_)|entry.clone()));
            let listing = FamilyListing { version: FAMILY_PROTOCOL,
                entries,limitations: limitations.clone() };
            if crate::screen::core::wire::to_json(&listing)?.len() <= crate::screen::core::wire::MAX_WINDOWS_BYTES {
                let pins = pinned.into_iter().map(|(_,pin)| (pin.lifetime.clone(),pin)).collect();
                self.owners.insert(owner.into(), pins);
                listing.validate()?;
                return Ok(listing);
            }
            if pinned.pop().is_none() { return Err(io::Error::other("family_listing_limits_exceed_frame")); }
            if !limitations.iter().any(|reason| reason == "family_list_frame_limit") {
                limitations.push("family_list_frame_limit".into());
            }
        }
    }
    pub(in crate::screen::os::host_worker_client) fn take(&mut self, owner: &str, source: &SourceId) -> io::Result<PinnedFamily> {
        let selected = selector(source).ok_or_else(|| io::Error::other("not_a_family_selector"))?;
        if let Some(pin)=self.owners.get_mut(owner).and_then(|pins|pins.remove(selected)) {return Ok(pin);}
        if let Some(registered)=self.registered.get_mut(owner) {
            let request=registered.iter().find(|(_, (entry,_,_))|selector(&entry.source)==Some(selected)).map(|(request,_)|request.clone());
            if let Some((_,pin,_))=request.and_then(|request|registered.remove(&request)) {return Ok(pin);}
        }
        Err(io::Error::new(io::ErrorKind::NotFound,"stale_or_foreign_family_selection"))
    }
    pub(in crate::screen::os::host_worker_client) fn receipt(&self,owner:&str,request:&str,pin:&crate::remote_window::WindowPin)->io::Result<Option<FamilyListEntry>> {
        match self.receipts.get(owner).and_then(|receipts|receipts.get(request)) {
            Some((entry,previous)) if previous==pin=>Ok(Some(entry.clone())),
            Some(_)=>Err(io::Error::new(io::ErrorKind::InvalidInput,"launch_request_pin_changed")),
            None=>Ok(None),
        }
    }
    pub(in crate::screen::os::host_worker_client) fn register(&mut self,owner:&str,request:&str,pin:crate::remote_window::WindowPin,raw:WindowList)->io::Result<FamilyListEntry> {
        if request.is_empty()||request.len()>128||request.chars().any(char::is_control) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,"invalid_launch_request_identity"));
        }
        if let Some(entry)=self.receipt(owner,request,&pin)? {return Ok(entry);}
        let mut adapter=crate::remote_window::os::adopt(&pin)?;
        let result=(|| {
            let root=adapter.inspect(&pin.binding)?;
            let metadata=raw.windows.iter().find(|window|window.id==pin.binding.native.0.to_string()&&window.app==root.app);
            let app_id=root.app_id.clone().or_else(||metadata.and_then(|window|window.app_id.clone()));
            let icon=metadata.and_then(|window|window.icon.clone()).filter(|icon|app_id.as_deref()==Some(icon.app_id.as_str()));
            let lifetime=token()?;
            let entry=FamilyListEntry {launch_request:Some(request.into()),source:SourceId::Window(format!("{FAMILY_PREFIX}{lifetime}")),
                identity:WindowIdentity {session:lifetime.clone(),generation:1},window:crate::screen::core::window_list::WindowEntry {
                    id:pin.binding.native.0.to_string(),title:root.title,app:root.app,app_id,icon,pid:None,
                    monitor:None,minimized:root.minimized,focused:root.focused}};
            let mut entries:Vec<_>=self.registered.get(owner).into_iter().flat_map(|pins|pins.values().map(|(entry,_,_)|entry.clone())).collect();
            entries.push(entry.clone());
            FamilyListing {version:FAMILY_PROTOCOL,entries,limitations:Vec::new()}.validate()?;
            Ok::<_,io::Error>((entry,lifetime))
        })();
        let (entry,lifetime)=match result {Ok(value)=>value,Err(error)=>{adapter.transfer_pin(&pin.binding);return Err(error);}};
        let binding=pin.binding.clone();
        self.receipts.entry(owner.into()).or_default().insert(request.into(),(entry.clone(),pin.clone()));
        self.registered.entry(owner.into()).or_default().insert(request.into(),(entry.clone(),PinnedFamily {
            adapter:std::sync::Arc::new(std::sync::Mutex::new(adapter)),binding,lifetime},pin));
        Ok(entry)
    }
}

