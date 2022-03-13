let args = discover::args_default();
args.rand_chaddr();
// args.giaddr = "192.168.0.1";
// args.params = "1,3,6";
print(args.to_string());
print(args.giaddr);
print(args.params);
print(args.chaddr);

let msg = args.send();
print(msg);

let req = request::args_default();
req.chaddr = args.chaddr;
req.req_addr = msg.yiaddr;
print(req.send());