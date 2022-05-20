# portguard

一个功能类似 `ssh -L` 与 `ssh -D` 的加密端口转发工具，但客户端**零配置**.

作者还是个新手，不为代码中可能出现的安全缺陷负责.

发现问题欢迎提 issue 和 pull request.

## 特色

- 客户端二进制文件是由服务端并自动生成，内置配置并由服务端配置反推出来，无需任何手动配置即可使用，而且只有生成的客户端才能与服务端通信（通过内置密钥对）.
- 无论客户端还是服务端，通信用到的所有密钥均为自动生成，无需在配置间复制粘贴.

## 工作原理

```
客户端 <-> 服务端 <-> 远端
```

1. 客户端绑定本地端口.
2. 服务端绑定公共端口.
3. 远端可以是服务端可达的某端口(google.com:443), 服务端的本地端口(127.0.0.1:port), 或者动态端口(通过内置 SOCKS5 服务实现).
4. 客户端与服务端通过`Noise_IK_25519_ChaChaPoly_BLAKE2s`协议握手.
5. 所有发往客户端本地端口的流量均由服务端转发至远端, 客户端与服务端之间的流量经过加密.

## 用法

1. 配置服务端的基本信息，存为 `config.toml`.

	Example:
	```
	host = '192.168.1.1'         # host of server
	port = 8022                  # port of server
	remote = '127.0.0.1:1080'    # default static remote (can be customized per client)
	# remote = 'socks5'          # or use dynamic remote
	```

2. 生成服务端密钥对, 运行 `portguard gen-key -c config.toml`.

	After that, `config.toml` becomes:
	```
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	```

2. 生成客户端二进制文件，运行 `portguard gen-cli -c config.toml -o pgcli`.

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

3. 服务端运行 `portguard server -c config.toml`

4. 客户端只需直接运行 `pgcli`，无需任何配置 (如果非要配置，可以配置本地端口和服务端地址，运行 `pgcli -p port -s saddr:sport`).

5. 所有的 TCP 流量都会像 `ssh -L` 或 `ssh -D` 那样被加密转发至 remote, 而且每个客户端可以在服务端配置各自的 remote.

## TODO

- [ ] I'm not familar with Noise protocol, now in my code every connection between client and server needs to handshake.
- [x] Set remote address per client.
- [ ] Improve performance.
- [ ] Test.

## Acknowledgement

本项目的开发离不开以下开源项目：

- [dend.ro's blog article about self-modify binary](https://blog.dend.ro/self-modifying-rust/), I learned how to modify binary.
- [snowstorm](https://github.com/black-binary/snowstorm), I use NoiseStream from this project for convenience
and add some code for timeout when reading from handshake message.
- [fast-socks5](https://github.com/dizda/fast-socks5), I use Socks5Socket from this library as a built-in SOCKS5 server.