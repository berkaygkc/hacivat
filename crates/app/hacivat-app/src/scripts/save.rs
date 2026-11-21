//! Saving a script tab (HV-SCRIPT-07, R-M4b-5, R-M4b-38): `file.save` and `file.save_all`,
//! the secret check first, then an atomic save that expects the file as it was read; a change
//! on disk asks "Overwrite?"; a failure keeps the tab dirty. Dirty is the tab's: the editor's
//! version against the version loaded, reloaded or saved.
//!
//! ## m4b-t01b-seams: nothing yet (Task 14); `file.save` and `file.save_all` are registered
//! disabled in [`super::install`].
