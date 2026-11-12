//! Step 0 of a connect: profile issues and option sanitizing (HV-CONN-01, HV-DRV-05). Body in
//! Task 16.

use hacivat_driver::url::FieldId;
use hacivat_driver::{EngineCatalog, ViolationReason};
use hacivat_net::TlsMode;

use crate::profile::Profile;

/// Why a profile cannot connect as it is (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IssueCode {
    NameMissing,
    HostMissing,
    PortInvalid,
    FileMissing,
    CreateAndReadOnly,
    TransportUnavailable,
    TlsCaUnreadable,
    TlsCertUnreadable,
    TlsKeyEncrypted,
    /// MySQL and MariaDB with `VerifyCa` in M2a (P11).
    TlsModeUnsupported {
        mode: TlsMode,
    },
    OptionRejected {
        key: String,
        reason: ViolationReason,
    },
}

/// One issue, with the field it belongs to (HV-CONN-01).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ValidationIssue {
    pub field: Option<FieldId>,
    pub code: IssueCode,
}

/// Every issue of `p` (Task 16). The stub finds none.
pub fn validate(_p: &Profile, _catalog: &EngineCatalog) -> Vec<ValidationIssue> {
    Vec::new()
}
