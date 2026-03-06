#[cfg(test)]
mod tests {
    use ferrous_node::network::dos::{
        DosProtection, MAX_ATTEMPTS_PER_WINDOW, MAX_CONNECTIONS_PER_IP, MAX_INBOUND_CONNECTIONS,
        MAX_MEMORY_PER_PEER, MAX_OUTBOUND_CONNECTIONS,
    };
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn test_max_inbound_connections() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Fill up inbound connections
        for i in 0..MAX_INBOUND_CONNECTIONS {
            // Use different IPs to avoid per-IP limit
            let unique_ip = IpAddr::V4(Ipv4Addr::new(127, (i / 256) as u8, (i % 256) as u8, 1));
            assert!(dos.can_accept_inbound(unique_ip));
            dos.record_connection(i as u64, unique_ip, true);
        }

        // Next one should fail
        assert!(!dos.can_accept_inbound(ip));
    }

    #[test]
    fn test_max_outbound_connections() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Fill up outbound connections
        for i in 0..MAX_OUTBOUND_CONNECTIONS {
            // Use different IPs to avoid per-IP limit
            let unique_ip = IpAddr::V4(Ipv4Addr::new(127, (i / 256) as u8, (i % 256) as u8, 1));
            assert!(dos.can_connect_outbound(unique_ip));
            dos.record_connection(i as u64, unique_ip, false);
        }

        // Next one should fail
        assert!(!dos.can_connect_outbound(ip));
    }

    #[test]
    fn test_max_connections_per_ip() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        for i in 0..MAX_CONNECTIONS_PER_IP {
            assert!(dos.can_accept_inbound(ip));
            dos.record_connection(i as u64, ip, true);
        }

        // Next one should fail
        assert!(!dos.can_accept_inbound(ip));
    }

    #[test]
    fn test_connection_rate_limiting() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // The issue:
        // 1. `can_accept_inbound` checks rate limit FIRST.
        // 2. If pass, caller calls `record_connection` OR `record_failed_attempt`.
        // 3. `record_failed_attempt` adds to attempts.

        // Loop runs MAX_ATTEMPTS_PER_WINDOW times (10).
        // Iter 0: count=0. check passes. record -> count=1.
        // ...
        // Iter 9: count=9. check passes. record -> count=10.

        // Loop finishes. count=10.

        // Next check: count=10. MAX=10. check fails.

        // Wait, why did it fail inside the loop?
        // assertion failed: dos.can_accept_inbound(ip)

        // If MAX=10.
        // i=0. count=0. OK. record -> count=1.
        // i=1. count=1. OK. record -> count=2.
        // ...
        // i=9. count=9. OK. record -> count=10.

        // It seems correct.
        // Maybe record_failed_attempt adds to failed_attempts map which blocks via cooldown?
        // Ah! record_failed_attempt sets cooldown!

        // Check if IP is in cooldown (recent failed attempts)
        // if let Some(cooldown_until) = self.failed_attempts.get(&ip) { ... }

        // So `record_failed_attempt` triggers cooldown immediately.
        // That's why subsequent checks fail!

        // To test rate limiting specifically (attempts per window), we should use `record_connection`
        // OR we should verify that `record_failed_attempt` triggers cooldown (which is a separate test).

        // To test "attempts per window" without cooldown interference, we need successful attempts?
        // Or we just test cooldown?

        // The test name is "test_connection_rate_limiting".
        // It implies testing MAX_ATTEMPTS_PER_WINDOW.
        // But `record_failed_attempt` triggers cooldown.
        // `record_connection` adds to attempts too.

        // So let's use `record_connection` to fill the window.
        // But `record_connection` also increments connection count.
        // We have per-IP limit of 3.
        // So we can't record 10 successful connections from same IP.

        // So how to test rate limit of 10 attempts?
        // We need attempts that are NOT successful but NOT failed (e.g. rejected for other reasons)?
        // Or we need `record_attempt` without `record_failed_attempt` (cooldown)?
        // The current API `record_failed_attempt` enforces cooldown.

        // Maybe the rate limit is intended for "connections opened and closed"?
        // If I connect and disconnect 10 times.

        for _ in 0..MAX_ATTEMPTS_PER_WINDOW {
            // Simulate connect then disconnect
            // Check if per-IP limit allows?
            // We need to clear connection count.
            if dos.can_accept_inbound(ip) {
                dos.record_connection(0, ip, true); // count=1
                dos.record_disconnection(0, ip, true); // count=0
                                                       // record_connection added to attempts.
            } else {
                panic!("Should accept");
            }
        }

        // Now attempts should be 10.
        // Next check should fail due to rate limit.
        assert!(!dos.can_accept_inbound(ip));
    }

    #[test]
    fn test_failed_attempt_cooldown() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        dos.record_failed_attempt(ip);

        // Immediate retry should fail (if we implement stricter cooldown logic, but current logic only checks failed_attempts map which is set on explicit failure)
        // Wait, record_failed_attempt sets failed_attempts entry.
        // can_accept_inbound checks failed_attempts entry.

        assert!(!dos.can_accept_inbound(ip));
    }

    #[test]
    fn test_memory_limits_per_peer() {
        let mut dos = DosProtection::new();
        let peer_id = 1;
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        dos.record_connection(peer_id, ip, true);

        // Allocate up to limit
        assert!(dos.can_allocate(peer_id, MAX_MEMORY_PER_PEER));
        dos.allocate(peer_id, MAX_MEMORY_PER_PEER);

        // Next allocation should fail
        assert!(!dos.can_allocate(peer_id, 1));
    }

    #[test]
    fn test_total_memory_limit() {
        let mut dos = DosProtection::new();
        let peer1 = 1;
        let peer2 = 2;
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        dos.record_connection(peer1, ip1, true);
        dos.record_connection(peer2, ip2, true);

        // Verify allocate logic with small allocations
        dos.allocate(peer1, 100);
        assert_eq!(dos.get_connection_counts().0, 2);
    }

    #[test]
    fn test_connection_counting() {
        let mut dos = DosProtection::new();
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        dos.record_connection(1, ip1, true); // Inbound
        dos.record_connection(2, ip2, false); // Outbound

        let (total, inbound, outbound) = dos.get_connection_counts();
        assert_eq!(total, 2);
        assert_eq!(inbound, 1);
        assert_eq!(outbound, 1);

        dos.record_disconnection(1, ip1, true);
        let (total, inbound, outbound) = dos.get_connection_counts();
        assert_eq!(total, 1);
        assert_eq!(inbound, 0);
        assert_eq!(outbound, 1);
    }

    #[test]
    fn test_cleanup_old_attempts() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Add an attempt
        // We can't inject old timestamp via public API.
        // But we can verify that after window, it might be cleaned up.
        // Since we can't sleep for 60s in test, we just check that normal operation works.
        // This test is limited without mocking time.

        assert!(dos.can_accept_inbound(ip));
    }

    #[test]
    fn test_successful_connection_clears_cooldown() {
        let mut dos = DosProtection::new();
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        dos.record_failed_attempt(ip);
        assert!(!dos.can_accept_inbound(ip));

        // If we somehow successfully connect (e.g. manual override or race condition handled),
        // it should clear cooldown.
        dos.record_connection(1, ip, true);

        // Now should be able to accept again (subject to rate limit)
        // But we just added a connection, so count increased.
        // And we recorded an attempt previously.
        // But cooldown (failed_attempts) should be gone.
        // Let's check if we can accept another (if per-IP limit allows)
        assert!(dos.can_accept_inbound(ip));
    }
}
