//! Quick fixes the error surfaces offer, and what each does (HV-CONN-03, Task 20).

/// A quick fix as the UI runs it (HV-CONN-03).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FixAction {
    EditConnection,
    TryAgain,
    ReenterPassword,
    UseDatabase(String),
    SupportedVersions,
    ConnectAnyway,
    EncryptWithoutVerifying,
}
