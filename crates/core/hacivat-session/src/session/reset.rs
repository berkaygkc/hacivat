//! Returning a session to a clean state between uses (HV-SESS-08; Task 7): the driver's
//! `RawConn::reset`, then the environment replay. A pool never reuses a session whose reset
//! failed (Q19: a borrowed default session is never reset). Empty until Task 7.
