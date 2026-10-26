//! Private Parent credit requests, reductions and retirement messages.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CreditWire {
    pub id: u64,
    pub source: String,
    pub generation: u64,
    pub total: u64,
    pub owners: [u64; OWNER_COUNT],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ImportNeedWire {
    source: String,
    connection: u64,
    object: u64,
    mapped: u64,
    metadata: u64,
}
impl ImportNeedWire {
    pub(crate) fn new(need: &super::super::capture_import::CaptureImportNeed) -> Self {
        Self {
            source: need.source().source().into(),
            connection: need.id().connection(),
            object: need.id().object(),
            mapped: need.mapped_bytes(),
            metadata: need.metadata_bytes(),
        }
    }
    pub(crate) fn need(
        &self,
        session: &str,
    ) -> io::Result<super::super::capture_import::CaptureImportNeed> {
        super::super::capture_import::CaptureImportNeed::new(
            SourceKey::new(session, &self.source)?,
            super::super::capture_import::CaptureImportId::new(self.connection, self.object)?,
            self.mapped,
            self.metadata,
        )
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum GrantRequest {
    FramePlan {
        request: u64,
        plan: PlanWire,
    },
    SupplementPlan {
        request: u64,
        image: u64,
        plan: PlanWire,
    },
    PreparedImport {
        request: u64,
        need: ImportNeedWire,
    },
    RegisterPrepared {
        request: u64,
        source: String,
        parent: Option<String>,
        selection: Option<SourceId>,
    },
    OwnPrepared {
        request: u64,
        source: String,
        parent: Option<String>,
        selection: Option<SourceId>,
    },
    BeginImports {
        request: u64,
        source: String,
    },
    EndImports {
        source: String,
        connection: u64,
    },
    EndPrepared {
        source: String,
    },
    EndPreparedOwner {
        source: String,
        owner: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub(crate) enum GrantValue {
    Frame(CreditWire),
    Supplement(CreditWire),
    Import(CreditWire),
    Registered,
    Connection(u64),
    PreparedOwner { source: String, owner: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GrantReply {
    pub request: u64,
    pub value: Option<GrantValue>,
    pub error: Option<String>,
    #[serde(default)]
    pub retryable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Reduction {
    pub id: u64,
    pub owners: [u64; OWNER_COUNT],
}
