# dhcpm

A cli tool for constructing & sending mocked dhcp client messages. `dhcpm` won't actually mess with the IP assigned to your network interfaces, it is only intended to mock messages to test dhcp servers. Aims to support v4 & v6, though v6 support is unfinished. Allows sending dhcp messages to arbitrary ports and ips.

## Sponsor

Thank you to [Bluecat](https://bluecatnetworks.com/) for sponsoring this work! `dhcpm` leverages [dhcproto](https://github.com/bluecatengineering/dhcproto) which is also worth checking out.

## Use

```
> dhcpm --help

Usage: dhcpm <target> [-b <bind>] [-p <port>] [-t <timeout>] [--output <output>] <command> [<args>]

dhcpm is a cli tool for sending dhcpv4/v6 messages

ex  dhcpv4:
        dhcpm 0.0.0.0 -p 9901 discover  (unicast discover to 0.0.0.0:9901)
        dhcpm 255.255.255.255 discover (broadcast discover to default dhcp port)
        dhcpm 192.168.0.1 dora (unicast DORA to 192.168.0.1)
        dhcpm 192.168.0.1 dora -o 118,C0A80001 (unicast DORA, incl opt 118:192.168.0.1)
    dhcpv6:
        dhcpm ::0 -p 9901 solicit       (unicast solicit to [::0]:9901)
        dhcpm ff02::1:2 solicit         (multicast solicit to default port)

Positional Arguments:
  target            ip address to send to

Options:
  -b, --bind        address to bind to [default: INADDR_ANY:0]
  -p, --port        which port use. [default: 67 (v4) or 546 (v6)]
  -t, --timeout     query timeout in seconds [default: 3]
  --output          select the log output format
  --help            display usage information

Commands:
  discover          Send a DISCOVER msg
  request           Send a REQUEST msg
  release           Send a RELEASE msg
  inform            Send a INFORM msg
  dora              Sends Discover then Request
  solicit           Send a SOLICIT msg (dhcpv6)
```

### Sending DHCP over arbitrary ports 

This will construct a discover message and unicast to `192.168.0.1:9901`:

```
dhcpm 192.168.0.1 -p 9901 discover
```

`dhpcm` will bind to `0.0.0.0:0` for replies, meaning the server must speak DHCP over arbitrary ports. To communicate over the standard ports, simply don't provide the `--port` option. `dhcpm` will then listen to the default port if you have suitable permissions.

```
dhcpm 192.168.0.1 discover
```

This will unicast to `192.168.0.1:67` and attempt to listen on `0.0.0.0:68`. You can change which address:port `dhcpm` listens on with the `--bind` option.


### Broadcast vs unicast 

To send a broadcast message (with the broadcast flag set) use the network broadcast address `255.255.255.255`.

```
dhcpm 255.255.255.255 discover
```

### Message parameters

Each sub-command (`discover`/`request`/`release`, etc) has sub-options. For example, by default dhcpm will use the default interfaces mac, you can override this by sending the appropriate Options

```
dhcpm 255.255.255.255 discover --chaddr "80:FA:5B:41:10:6B"
```

