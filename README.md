# portguard

An encrypted port forwarding tool that works like `ssh -L` and `ssh -D`, but **Zero Config** for client.

It is currently a simple project and the author is not familiar with security, we take no responsibility for any security flaws.

Welcome to create issues and pull requests.

[中文介绍](https://github.com/wlh320/portguard/blob/master/README_zh.md)

## Features

- Client's binary executable is auto generated from server, user can run it **without any config by hand**, and only generated clients can communicate with server for auth.
- Every DH key used is auto generated too, without any copy-and-paste of config files.

## How it works

```
client <-> server <-> remote
```

1. Client listens on local port.
2. Server listens on public port.
3. Remote can be a remote port (google.com:443), server's local port (127.0.0.1:xxxx), or dynamic (using a built-in SOCKS5 server).
4. Client and server handshake using `Noise_IK_25519_ChaChaPoly_BLAKE2s`.
5. Client's local port is forwarded to remote port by server, and traffic between client and server is *encrypted*.

## Usage

1. Config server with a `config.toml` file.

	Example:
	```
	host = '192.168.1.1'         # host of server
	port = 8022                  # port of server
	remote = '127.0.0.1:1080'    # default static remote (can be customized per client)
	# remote = 'socks5'          # or use dynamic remote
	```

2. Generate server keypair by running `portguard gen-key -c config.toml`.

	After that, `config.toml` becomes:
	```
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	```

2. Generate client binary executable by running `portguard gen-cli -c config.toml -o pgcli`.

	After that, `config.toml` becomes:
	```
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	[clients."KhM4xjza7I8gD7U3uQGuTZ73fIU+Zi66QJzPhmLFJQ0="]
	name = 'user'
	pubkey = 'KhM4xjza7I8gD7U3uQGuTZ73fIU+Zi66QJzPhmLFJQ0='
	```

	And a client binary executable is output to `pgcli`

3. Run `portguard server -c config.toml` on server

4. Run `pgcli` on client without any configs
(local port or server address can be customized with `pgcli -p port -s saddr:sport` if you like).

5. All TCP traffic to client's local port is forwarded to remote by server with encryption.

## TODO

- [ ] I'm not familar with Noise protocol, now in my code every connection between client and server needs to handshake.
- [x] Set remote address per client.
- [ ] Improve performance.
- [ ] Test.

## Changelog

### v0.2.1`
- add x86_64-apple-darwin support (not tested)
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
- [snowstorm](https://github.com/black-binary/snowstorm), I use NoiseStream from this project for convenience
and add some code for timeout when reading from handshake message.
- [fast-socks5](https://github.com/dizda/fast-socks5), I use Socks5Socket from this library as a built-in SOCKS5 server.
