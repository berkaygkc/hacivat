//! The SSH transport step (Task 14, B11, Q44): at order 40, for a `TransportSpec::SshTunnel`
//! profile, it acquires a `TunnelLease` from `ConnectDeps::tunnels` (outside the connect clock),
//! skips Resolve (the bastion resolves the database host), classifies the address with
//! `ssh_address_class`, and hands the driver the lease's factory. Empty until Task 14: an SSH
//! profile is still refused at Validate (`IssueCode::TransportUnavailable`).
