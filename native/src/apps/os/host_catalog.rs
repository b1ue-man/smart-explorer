//! Installed catalog snapshots, icon identity and exact request facts.
use super::*;

impl<P: HostPlatform> AppHost<P> {
    pub(crate) fn with_icons(mut self, icons: CatalogIcons) -> Self {
        self.icons = Some(icons);
        self
    }
    pub(super) fn snapshot(&self) -> io::Result<(CatalogSnapshot, PlatformCatalog<P::Target>)> {
        let mut native = self.platform.snapshot()?;
        native.apps.sort_by(|a, b| a.app.app_id.cmp(&b.app.app_id));
        if native
            .apps
            .windows(2)
            .any(|a| a[0].app.app_id == a[1].app.app_id)
        {
            return Err(io::Error::other("ambiguous installed application identity"));
        }
        let mut hash = Sha256::new();
        // Lock/focus changes require fresh admission, but are not a new login or
        // installed catalog. A local unlock can continue the SAME request ID.
        hash.update(
            serde_json::to_vec(&(
                &self.device,
                &native.session.user_id,
                &native.session.session_id,
            ))
            .map_err(io::Error::other)?,
        );
        for app in &native.apps {
            hash.update(
                serde_json::to_vec(&(
                    &app.app.app_id,
                    &app.app.name,
                    &app.fingerprint,
                    app.app.can_launch,
                ))
                .map_err(io::Error::other)?,
            );
        }
        let revision = format!("{:x}", hash.finalize());
        let authorization_scope = SessionId::try_from(format!("app-catalog:{revision}"))?;
        let mut catalog = CatalogSnapshot {
            version: 1,
            device_id: self.device.clone(),
            session: native.session.clone(),
            revision,
            authorization_scope,
            apps: native.apps.iter().map(|a| a.app.clone()).collect(),
            limits: native.limits.clone(),
        };
        if let Some(icons) = &self.icons {
            (icons.references)(&mut catalog.apps);
        }
        for app in &mut catalog.apps {
            if app
                .icon
                .as_ref()
                .is_some_and(|icon| icon.validate().is_err() || icon.app_id != app.app_id.as_str())
            {
                app.icon = None;
            }
        }
        Ok((catalog, native))
    }
    pub(crate) fn catalog(&self) -> io::Result<CatalogSnapshot> {
        self.snapshot().map(|s| s.0)
    }
    /// Main authorizes catalog/icon disclosure before invoking this local read.
    pub(crate) fn icon(&self, id: &AppId, expected: &IconRef) -> io::Result<Option<IconReply>> {
        let catalog = self.catalog()?;
        let app = catalog
            .apps
            .iter()
            .find(|a| &a.app_id == id && a.icon.as_ref() == Some(expected))
            .ok_or_else(|| invalid("icon no longer belongs to current installed catalog"))?;
        let reply = if let Some(icons) = &self.icons {
            (icons.content)(app, expected)?
        } else {
            self.platform.icon(app)?
        };
        if let Some(reply) = &reply {
            reply.bytes_for(expected)?;
        }
        Ok(reply)
    }

    pub(super) fn facts(
        &self,
        request: &LaunchRequest,
        catalog: &CatalogSnapshot,
    ) -> Result<LaunchFacts, String> {
        if request.catalog_revision.len() != 64
            || !request
                .catalog_revision
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid_catalog_revision".into());
        }
        if request.target_device_id != self.device {
            return Err("target_device_mismatch".into());
        }
        if request.user_id != catalog.session.user_id
            || request.session_id != catalog.session.session_id
        {
            return Err("host_user_or_login_session_changed".into());
        }
        if request.catalog_revision != catalog.revision {
            return Err("host_catalog_changed".into());
        }
        let app = catalog
            .apps
            .iter()
            .find(|a| a.app_id == request.app_id)
            .ok_or_else(|| "app_not_in_current_host_catalog".to_string())?;
        if !catalog.session.interactive {
            return Err("host_has_no_authorized_interactive_session".into());
        }
        Ok(LaunchFacts {
            request: request.clone(),
            catalog: catalog.clone(),
            app: app.clone(),
            authorization_scope: catalog.authorization_scope.clone(),
        })
    }
}
