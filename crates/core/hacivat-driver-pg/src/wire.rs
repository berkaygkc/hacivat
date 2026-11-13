//! The wire observer (Task 10, B2, B3, B8, B9, Q11, Q12): a layer on the decrypted side of the
//! injected stream that reads backend messages as they pass (`BackendKeyData` for the PID,
//! `ReadyForQuery` for the transaction status, the authentication request for the insecure-auth
//! guard and channel binding). Empty until Task 10.
