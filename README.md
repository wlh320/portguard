# portguard

An encrypted port forwarding tool that just works like ssh tunnel, but **Zero Config** for client.

It is currently a simple project and the author is not familiar with security, we take no responsibility for any security flaws.

Welcome to create issues and pull requests.

[中文介绍](https://github.com/wlh320/portguard/blob/master/README_zh.md)

## Use case

- You need to expose local port to public ip with encryption and authorization.
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
2. Remote can be a remote port (google.com:443), a local port (127.0.0.1:xxxx), or dynamic.
3. Client works in any of the following modes:
	- `ssh -L` mode: visit static port of remote2 through server.
	- `ssh -D` mode: visit dynamic remote2 through server's builtin socks5 server.
	- `ssh -R` mode: expose port of remote1 to server and register a _service id_.
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
	# to generate this, run: ./portguard gen-cli -c config.toml -o cli -t 127.0.0.1:2333
	[clients."qFGPs28K1hshENagjW3aKVXn4NrB7X2jftBue3SLRW0="]
	name = 'proxy'
	pubkey = 'qFGPs28K1hshENagjW3aKVXn4NrB7X2jftBue3SLRW0='
	remote = '127.0.0.1:2333'

	# works like ssh -D
	# to generate this, run: ./portguard gen-cli -c config.toml -o cli_socks5 -t socks5
	[clients."AIVbWCQQ0+VawQZk/AVjq2Ix9SagngxGXtEK26AUa3U="]
	name = 'proxy_socks'
	pubkey = 'AIVbWCQQ0+VawQZk/AVjq2Ix9SagngxGXtEK26AUa3U='
	remote = 'socks5'

	# works like ssh -R
	# to generate this, run: ./portguard gen-cli -c config.toml -o rclient -s 1 -t 127.0.0.1:8000
	[clients."h6M/DaKv5IOMM4Y2dkiZKpudQ5BCO5DvnNNWqZczGXs="]
	name = 'reverse proxy client'
	pubkey = 'h6M/DaKv5IOMM4Y2dkiZKpudQ5BCO5DvnNNWqZczGXs='
	remote = ['127.0.0.1:8000', 1]

	# in order to connect port exposed by ssh -R
	# to generate this, run: ./portguard gen-cli -c config.toml -o rvisitor -s 1
	[clients."Q5VqAyS9dl0CSrOnWOB9XmI0Kb1X83FL6iee3Iio9ls="]
	name = 'reverse proxy visitor'
	pubkey = 'Q5VqAyS9dl0CSrOnWOB9XmI0Kb1X83FL6iee3Iio9ls='
	remote = 1
	```

3. Run `portguard server -c config.toml` on server side.

4. Run generated binary on client side without any configs
(local port or server address can be customized with `pgcli -p port -s saddr:sport` if you like).

## TODO

- [ ] I'm not familar with Noise protocol, now in my code every connection between client and server needs to handshake (except reverse proxy mode).
- [x] Set remote address per client.
- [ ] Benchmark and improve performance.
- [ ] Test.

## Changelog

### v0.3.0-pre
- add `ssh -R` feature using yamux (It just works, recommend to use existing projects like frp or rathole with `-L` mode)
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
- [snowstorm](https://github.com/black-binary/snowstorm), I use NoiseStream from this project for convenience
and add some code for timeout when reading from handshake message.
- [fast-socks5](https://github.com/dizda/fast-socks5), I use Socks5Socket from this library as a built-in SOCKS5 server.
- [rust-yamux](https://github.com/libp2p/rust-yamux), I use yamux from this library to impl reverse proxy. .
