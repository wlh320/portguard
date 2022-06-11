# portguard

A port forwarding tool with encryption and authentication that just works like ssh tunnel, but **Zero Config** for client.

**Warning** It is currently a simple project and the author is not familiar with security, we take no responsibility for any security flaws.

Welcome to create issues and pull requests.

[中文介绍](https://github.com/wlh320/portguard/blob/master/README_zh.md)

## Use case

- You need to expose local port to public ip with encryption, and you just want specific users to visit it.
- You don't like teaching your users how to config the client program.

## Features

- Works just like ssh tunnel, but using `Noise` protocol.
- Client's binary executable is auto generated from server, user can run it **without any config by hand**, and only generated clients can communicate with server for auth.
- Every DH key used is auto generated too, without any copy-and-paste of config files.

## How it works

```
remote1 <-> client <-> server <-> remote2
```

1. Server listens on public IP and a public port.
2. Remote can be a remote port (google.com:443), a local port (127.0.0.1:xxxx), or dynamic (socks5).
3. Client works in any of the following modes:
	- `ssh -L` mode: visit static port of remote2 through server.
	- `ssh -D` mode: visit dynamic remote2 through server's builtin socks5 server.
	- `ssh -R` mode: expose remote1 (port or dynamic) to server and register a _service id_.
	- `ssh -R visitor` mode: only clients in this mode with same _service id_ can visit the exposed port.
4. Client and server handshake using `Noise_IK_25519_ChaChaPoly_BLAKE2s`.
5. Data transferred with encryption between client and server.

## Usage

1. Config server with a `config.toml` file.

	Example:
	```toml
	host = '192.168.1.1'         # host of server
	port = 8022                  # port of server
	remote = '127.0.0.1:1080'    # default static remote (can be customized per client)
	# remote = 'socks5'          # or use dynamic remote
	```

2. Generate server keypair by running `portguard gen-key -c config.toml`.

	After that, `config.toml` becomes:
	```toml
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	```

3. Generate client binary executable using `portguard gen-cli` subcommand in 4 different modes:

	```
	USAGE:
	    portguard gen-cli [OPTIONS] --config <CONFIG> --output <OUTPUT>

	OPTIONS:
	    -c, --config <CONFIG>      location of config file
	    -h, --help                 Print help information
	    -i, --input <INPUT>        location of input binary (current binary by default)
	    -n, --name <NAME>          name of client [default: user]
	    -o, --output <OUTPUT>      location of output binary
	    -s, --service <SERVICE>    service id of a reverse proxy
	    -t, --target <TARGET>      client's target address, can be socket address or "socks5"
	```

	Example of generated config file:

	```toml
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='

	# works like ssh -L
	# to generate this, run: ./portguard gen-cli -c config.toml -o client -t 127.0.0.1:2333
	# `name` field does nothing to auth, just for admin of server to distinguish clients
	[[clients]]
	name = "normal"
	pubkey = "dnso7kN2vhgLR/DVcAJRy1c9lRns3w7ESfB42szQWVI="
	remote = "127.0.0.1:2333"

	# works like ssh -D
	# to generate this, run: ./portguard gen-cli -c config.toml -o client_socks5 -t socks5
	[[clients]]
	name = "socks5"
	pubkey = "+iOiRpafA8/QKVclKZHiRkDSAQv4USkuS5qFJWOT/wk="
	remote = "socks5"

	# works like ssh -R
	# to generate this, run: ./portguard gen-cli -c config.toml -o rclient -s 1 -t 127.0.0.1:2333
	[[clients]]
	name = "rclient"
	pubkey = "kJqUC1fRRD9DW24zBmOkEKdEIX/EoSjfMeLxw2QvETI="
	hash = "6jgZoM/RyNHG7QxzLwcij32RjFYHGOGIsUBGG9n9ah8="
	remote = ["127.0.0.1:2333", 1]

	# in order to connect port exposed by ssh -R
	# to generate this, run: ./portguard gen-cli -c config.toml -o rvisitor -s 1
	[[clients]]
	name = "rvisitor"
	pubkey = "t+Zb+pfnQ3aIaJZfz0wnnjrUNcW4t8HPzOYf7gEhURc="
	remote = 1

	# works like ssh (-R + -D)
	# to generate this, run: ./portguard gen-cli -c config.toml -o rclient -s 2 -t socks5
	[[clients]]
	name = "rclient_socks5"
	pubkey = "DHfFF3G+KFMHZjEiUwmTEo5+C2WZCtN+M0rirkgX/2c="
	hash = "I4Ws+fmbuYEVc+zux8IqreY02EPw5KFuOx/hLDirH5s="
	remote = ["socks5", 2]

	# same as "rvisitor"
	[[clients]]
	name = "rvisitor_socks5"
	pubkey = "vmdp+x5bhUkZKA3SGqA5Gv+VX8/XfutzrAfGxk+Q3zo="
	remote = 2
	```

3. Run `portguard server -c config.toml` on server side.

4. Run generated binary on client side without any configs
(local port or server address can be customized with `portguard client -p port -s saddr:sport` if you like).

Suggestions:
- (since v0.3.1) When generating clients, use `pgcli` as input file to reduce file size (size of client is about 2MB).
- Can compress generated clients using `upx`, but the builtin config of client after compressed is unchangeable (700kB after compressed).

## TODO

- [x] ~~I'm not familar with Noise protocol, now in my code every connection between client and server needs to handshake (except reverse proxy mode).~~ Now I think it is a feature.
- [x] Set remote address per client.
- [ ] Benchmark and improve performance.
- [ ] When will a connection be closed? Put it in logs.
- [ ] Test.
- [ ] UDP ?

## Changelog

### v0.3.2
- add `aarch64-linux-android` support (both binary and JNI lib, tested on my own phone).
- add a new subcommand `clone-cli` to clone existing clients to other platform with built-in config unchanged.
- better error handling for `ssh -R` server.

### v0.3.1
- before starting proxying, server will check filehash of reverse proxy client.
- add a minimal client-only binary named `pgcli` for reducing file size in client side.
- add a new subcommand `mod-cli` to re-generate existing client's keypair.
- change default listening port of client to `8022`

### v0.3.0
- `--reverse` arguments is removed for client because role of client can be detected automatically.
- clients in server config are now represented as set rather than map.

### v0.3.0-pre2
- add `ssh -R` feature using yamux (It just works, recommend to use existing projects like frp or rathole with `-L` mode)
- add `ssh -R` + `ssh -D` feature (socks5 reverse proxy)
- more tests needed

### v0.2.1
- add `x86_64-apple-darwin` support (not tested)
- regularize section name
- server can generate client for any platform (windows, linux, macos)
- client can derive its public key using list-key subcommand

### v0.2.0
- add `ssh -D` feature with a built-in SOCKS5 server
- can overwrite config of existing client

### v0.1.0
- basic `ssh -L` feature

## Acknowledgement

Thanks for these projects:

- [dend.ro's blog article about self-modify binary](https://blog.dend.ro/self-modifying-rust/), I learned how to modify binary.
- [snowstorm](https://github.com/black-binary/snowstorm), I use NoiseStream from this project for convenience and add some code for timeout when reading from handshake message.
- [fast-socks5](https://github.com/dizda/fast-socks5), I use Socks5Socket from this library as a built-in SOCKS5 server.
- [rust-yamux](https://github.com/libp2p/rust-yamux), I use yamux from this library for TCP stream multiplexing in reverse proxy.
