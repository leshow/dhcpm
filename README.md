# dhcpm

A cli tool for constructing & sending dhcp client messages. Aims to support v4 & v6. Allows sending dhcp messages to arbitrary ports and ips. Right now only unicast, but (hopefully) soon broadcast also.

## Use

Send a v4 discover message to 0.0.0.0:9901

```
dhcpm 0.0.0.0 -p 9901 discover
```

`dhcpm` will decide which protocol you use based on your target address. So use ipv6 for dhcpv6 and ipv4 for dhcpv4.
