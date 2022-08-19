# dhcpm

## Sponsor

Thank you to [Bluecat](https://bluecatnetworks.com/) for sponsoring this work! `dhcpm` leverages [dhcproto](https://github.com/bluecatengineering/dhcproto) check that out for the DHCP protocol.

## About

A cli tool (and dhcp script runner!) for constructing & sending mocked dhcp client messages. `dhcpm` won't actually mess with the IP assigned to your network interfaces, it is intended to mock dhcp messages for testing dhcp servers. It aims to support v4 & v6, though v6 support is unfinished. Allows sending dhcp messages to non-default ports, and can be scripted with [rhai](https://github.com/rhaiscript/rhai).

This software is a personal project and should be considered beta. I use the basic cli features often, but the scripting features are new.

## Installation

You can install with

```
cargo install dhcpm
```

To include the rhai scripting feature, add

```
cargo install dhcpm --feautures "script"
```

## Use

```
> dhcpm --help

Usage: dhcpm <target> [-b <bind>] [-i <interface>] [-p <port>] [-t <timeout>] [--output <output>] [--script <script>] [--no-retry <no-retry>] [<command>] [<args>]

dhcpm is a cli tool for sending dhcpv4/v6 messages

ex  dhcpv4:
        dhcpm 255.255.255.255 discover          (broadcast discover to default dhcp port)
        dhcpm 192.168.0.255 discover            (broadcast discover on interface bound to 192.168.0.x)
        dhcpm 0.0.0.0 -p 9901 discover          (unicast discover to 0.0.0.0:9901)
        dhcpm 192.168.0.1 dora                  (unicast DORA to 192.168.0.1)
        dhcpm 192.168.0.1 dora -o 118,C0A80001  (unicast DORA, incl opt 118:192.168.0.1)
    dhcpv6:
        dhcpm ::0 -p 9901 inforeq       (unicast inforeq to [::0]:9901)
        dhcpm ff02::1:2 inforeq         (multicast inforeq to default port)

Positional Arguments:
  target            ip address to send to

Options:
  -b, --bind        address to bind to [default: INADDR_ANY:0]
  -i, --interface   interface to use (requires root or `cap_net_raw`) [default:
                    None - selected by OS]
  -p, --port        which port use. [default: 67 (v4) or 546 (v6)]
  -t, --timeout     query timeout in seconds [default: 5]
  --output          select the log output format (json|pretty|debug) [default:
                    pretty]
  --no-retry        setting to "true" will prevent re-sending if we don't get a
                    response [default: false]
  --help            display usage information

Commands:
  discover          Send a DISCOVER msg
  request           Send a REQUEST msg
  release           Send a RELEASE msg
  inform            Send an INFORM msg
  decline           Send a DECLINE msg
  dora              Sends Discover then Request
  inforeq           Send a INFORMATION-REQUEST msg (dhcpv6)
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

### Using specific interface

You can pass the `--interface/-i` param to bind to a specific interface by name, for example `--interface enp6s0`. Using this, you will only receive/send responses over that device. Ex,

```
dhcpm 255.255.255.255 -i enp6s0 discover --chaddr random
```

You can also use `ip addr` on linux to get the broadcast address of a particular interface:

```
2: enp6s0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc mq state UP group default qlen 1000
    link/ether xx:xx:xx:xx:xx:xx brd ff:ff:ff:ff:ff:ff
    inet 192.168.0.130/24 brd 192.168.0.255 scope global noprefixroute enp6s0
```

Note `brd 192.168.0.255`. You can pass this to `dhcpm` and the kernel should select that interface to broadcast on (`enp6s0` in this example)

```
dhcpm 192.168.0.255 discover
```

### Message parameters

Each sub-command (`discover`/`request`/`release`, etc) has sub-options. For example, by default dhcpm will use the default interfaces mac, you can override this by sending the appropriate Options

```
dhcpm 255.255.255.255 discover --chaddr "80:FA:5B:41:10:6B"
```

### dhcpv6

With DHCPv6, many messages are sent on the multicast group `ff02::1:2` but responses are often unicast back on link-local addresses (starting with `fe80`). `dhcpm` won't be able to receive this data if you've got another dhcpv6 client listening on `[::0]:546`, the dhcpv6 client port. The other process is will likely read the datagram first.

For example, my box has:

```
> sudo lsof -Pi UDP
...
NetworkMa     711            root   20u  IPv6 12173080      0t0  UDP leshowbox:546
```

Listening on this `[::0]:546`, so that process would need to be killed before `dhcpm` could print a reply. Still, I have often found it enough to use `dhpcm` to generate a message, then look at the response in wireshark or tcpdump to inspect its validity.

Specify an interface with v6, it is necessary to join the multicast group.

```
> sudo dhcpm ff02::1:2 -i enp6s0 inforeq
```

### Scripting

Scripting support with [rhai](https://github.com/rhaiscript/rhai). Compile `dhcpm` with the `script` feature and give it a path with `--script`:

```
dhcpm 255.255.255.255 --script test.rhai
```

In the script, you can create new discover arguments with:

```
let args = discover::args_default();
```

You can send this message with `args.send()`.

Message types supported in script are:

- `discover::args_default()`
- `request::args_default()`
- `release::args_default()`
- `inform::args_default()`

Be careful about what scripts you choose to run, especially if you use ports only accessible with `sudo`, as the scripts arbitrary code will be executed with whatever permissions you give it.
