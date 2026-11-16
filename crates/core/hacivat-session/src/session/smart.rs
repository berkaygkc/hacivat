// ## m3b-t01c-core-seams
//! Smart mode (HV-EXEC-10, D-036; Task 19): Auto, but a unit whose effects open a write
//! (`SessionEffects::OPENS_WRITE`, from the exec gate) opens the user's transaction first,
//! through `manual_begin`.
//!
//! STUB until Task 19: `TxCommand::SetMode(Smart)` still answers `TxError::NeedsClassifier`
//! and a profile's Smart mode starts as Manual (the actor), as in M3a. Nothing here yet.
