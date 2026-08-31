//! Vite-style startup banner: prints the local and LAN-reachable addresses
//! a server is listening on, one line per network interface.

/// Print a banner for a server listening on `host:port`.
///
/// `scheme` is prefixed to each URL (e.g. `"http://"`, or `""` for a bare
/// `host:port` when the address isn't a browsable URL, like raw gRPC).
/// `path` is appended after the port (e.g. `"/"`).
///
/// Network lines are only printed when `host` binds all interfaces
/// (`0.0.0.0` / `::`) -- otherwise the LAN addresses wouldn't actually be
/// reachable on that server.
pub fn print(label: &str, host: &str, port: u16, scheme: &str, path: &str) {
    let binds_all_interfaces = matches!(host, "0.0.0.0" | "::" | "[::]");

    println!();
    println!("  ➜  {label}");

    let local_host = if binds_all_interfaces { "localhost" } else { host };
    println!("     Local:    {scheme}{local_host}:{port}{path}");

    if binds_all_interfaces {
        match if_addrs::get_if_addrs() {
            Ok(interfaces) => {
                for iface in interfaces {
                    if iface.is_loopback() || !iface.addr.ip().is_ipv4() {
                        continue;
                    }
                    println!(
                        "     Network:  {scheme}{}:{port}{path}  {}",
                        iface.addr.ip(),
                        iface.name
                    );
                }
            }
            Err(e) => warn!("Failed to enumerate network interfaces: {}", e),
        }
    }

    println!();
}
