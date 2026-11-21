//! The connection chooser (HV-SCRIPT-09, R-M4b-9, Q14; NOTES surface 7): after a pick in Go to
//! script, and from "Open under another connection…", it lists only the connections where the
//! file is visible (binding and environment scope), the active tab's connection first, then
//! the least severe environment; **production is never preselected** unless it is the active
//! tab's connection, and the connection a tab or row is under is never offered for it.
//!
//! ## m4b-t01b-seams: nothing yet (Task 15).
