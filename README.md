# portguard

encrypted port forwarding that works like `ssh -L`, but **Zero Config** for client.

It is a simple project and the author is not familiar with security, we take no responsibility for any security flaws. 
Welcome to create issues and pull requests.

## Features

- Client binary is auto generated **without any config by hand**, and only generated clients can communicate with server for auth.
- Every DH key used is auto generated too.

## How it works

```
client <-> server <-> remote
```

1. client listen on local port.
2. server listen on public port.
3. client and server handshake using `Noise_IK_25519_ChaChaPoly_BLAKE2s`.
3. proxy local port to remote port set in server config, traffic between client and server is *encrypted*.

## Usage

1. Config server with a `config.toml` file.

	Example:
	```
	host = '192.168.1.1'
	port = 6000
	remote = '127.0.0.1:1080'
	```

2. Generate server keypair using `./portguard gen-key -c config.toml`.

	After that, `config.toml` becomes:
	```
	host = '192.168.1.1'
	port = 6000
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	```

2. Generate client binary using `./portguard gen-cli -c config.toml -o pgcli`.

	After that, `config.toml` becomes:
	```
	host = '192.168.1.1'
	port = 6000
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	[clients."KhM4xjza7I8gD7U3uQGuTZ73fIU+Zi66QJzPhmLFJQ0="]
	name = 'user'
	pubkey = 'KhM4xjza7I8gD7U3uQGuTZ73fIU+Zi66QJzPhmLFJQ0='
	```

	And a client binary is output to `pgcli`

3. Server run `./portguard server -c config.toml`

4. Client just run `./pgcli` without any configs
(local port can be customized with `./pgcli -p port` if you like).

5. All traffic to client's local port is proxied to remote by server with encryption.

## TODO

- [ ] I'm not familar with Noise protocol, now in my code every connection between client and server needs to handshake.
- [ ] Set remote address per client.
- [ ] Plan to use other Noise implementation.

## Acknowlegement

Thank for these projects:

- [dend.ro's blog article about self-modify binary](https://blog.dend.ro/self-modifying-rust/), I learned how to modify binary.
- [Snowstorm](https://github.com/black-binary/snowstorm), I use NoiseStream in this project and add some code for timeout when reading from handshake message.
