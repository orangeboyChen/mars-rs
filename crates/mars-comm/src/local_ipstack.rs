//! `mars/comm/socket/local_ipstack.h` — `TLocalIPStack`.
//!
//! `local_ipstack_detect()` walks the platform's interfaces (and, on the
//! Apple platforms, asks the system what the current network can carry), so
//! there is no counterpart for it here: whichever stack the host detected is
//! handed to the code that needs it, which is [`LocalIpStack`] as an
//! argument.

/// `TLocalIPStack` — what the local network carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocalIpStack {
    /// `ELocalIPStack_None` — no network at all.
    #[default]
    None = 0,
    /// `ELocalIPStack_IPv4`.
    IPv4 = 1,
    /// `ELocalIPStack_IPv6` — IPv6 only, which is the only stack NAT64
    /// applies to.
    IPv6 = 2,
    /// `ELocalIPStack_Dual`.
    Dual = 3,
}

impl LocalIpStack {
    /// `TLocalIPStackStr[...]` — the name the C++ logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "ELocalIPStack_None",
            Self::IPv4 => "ELocalIPStack_IPv4",
            Self::IPv6 => "ELocalIPStack_IPv6",
            Self::Dual => "ELocalIPStack_Dual",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_the_ones_the_cpp_logs() {
        assert_eq!(LocalIpStack::None.name(), "ELocalIPStack_None");
        assert_eq!(LocalIpStack::IPv4.name(), "ELocalIPStack_IPv4");
        assert_eq!(LocalIpStack::IPv6.name(), "ELocalIPStack_IPv6");
        assert_eq!(LocalIpStack::Dual.name(), "ELocalIPStack_Dual");
    }

    #[test]
    fn the_discriminants_are_the_ones_of_the_c_enum() {
        assert_eq!(LocalIpStack::None as i32, 0);
        assert_eq!(LocalIpStack::IPv4 as i32, 1);
        assert_eq!(LocalIpStack::IPv6 as i32, 2);
        assert_eq!(LocalIpStack::Dual as i32, 3);
        // and nothing is detected until the host says so
        assert_eq!(LocalIpStack::default(), LocalIpStack::None);
    }
}
