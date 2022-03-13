let args = discover::args_default();
args.rand_chaddr();
args.giaddr = "192.168.0.1";
args.params = "1,3,6";
print(args.giaddr);
print(args.params);
print(args.chaddr);