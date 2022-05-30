# portguard

一个功能类似 ssh 隧道 的加密端口转发工具，但客户端**零配置**.

作者还是个新手，不为代码中可能出现的安全缺陷负责.

发现问题欢迎提 issue 和 pull request.

## 用例

- 你有一个公网 ip, 并且你想把一个本地端口暴露在公网，但只希望特定用户能访问
- 你不想花时间教给这些特定用户如何配置才能访问这个端口，只想扔给他们一个可执行文件

## 特色

- 与 ssh 隧道一样的工作方式，但加密传输用的是 noise 协议.
- 客户端二进制文件是由服务端并自动生成，内置配置并由服务端配置反推出来，无需任何手动配置即可使用，而且只有生成的客户端才能与服务端通信（通过内置密钥对）.
- 无论客户端还是服务端，通信用到的所有密钥均为自动生成，无需在配置间复制粘贴.

## 工作原理

```
远端1 <-> 客户端 <-> 服务端 <-> 远端2
```

1. 服务端绑定公网IP的一个端口.
2. 远端可以是其他的公网端口(google.com:443), 本地端口(127.0.0.1:port), 或者动态端口.
3. 客户端可以工作于以下任一模式：
	- `ssh -L`模式：通过服务端访问远端2的静态端口。
	- `ssh -D`模式：通过服务端内置的socks5服务访问动态的远端2。
	- `ssh -R`模式：将远端1的端口暴露给服务端并注册一个 _service id_。
	- `ssh -R visitor` 模式：只有在此模式下具有相同 _service id_ 的客户端才能访问暴露的端口。
4. 客户端与服务端通过`Noise_IK_25519_ChaChaPoly_BLAKE2s`协议握手.
5. 随后客户端与服务端之间的流量加密传输.

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

	随后, `config.toml` 变成了:
	```
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='
	```

2. 生成客户端二进制文件，运行 `portguard gen-cli` 子命令.

	命令参数：
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

	生成各种客户端的例子（配置和可执行文件均由命令生成，无需手动编辑）:

	```toml
	host = '192.168.1.1'
	port = 8022
	remote = '127.0.0.1:1080'
	pubkey = '1y3HW8TDxChtke5nyEdLGj+OkQSg8JjLdalSHzD+aWI='
	prikey = 'eHg7jR/IZwEZEqeyR27IUTN0py5a3+wP0uM+z9HeWn8='

	# ssh -L 模式的客户端
	# to generate this, run: ./portguard gen-cli -c config.toml -o cli -t 127.0.0.1:2333
	[clients."qFGPs28K1hshENagjW3aKVXn4NrB7X2jftBue3SLRW0="]
	name = 'proxy'
	pubkey = 'qFGPs28K1hshENagjW3aKVXn4NrB7X2jftBue3SLRW0='
	remote = '127.0.0.1:2333'

	# ssh -D 模式的客户端
	# to generate this, run: ./portguard gen-cli -c config.toml -o cli_socks5 -t socks5
	[clients."AIVbWCQQ0+VawQZk/AVjq2Ix9SagngxGXtEK26AUa3U="]
	name = 'proxy_socks'
	pubkey = 'AIVbWCQQ0+VawQZk/AVjq2Ix9SagngxGXtEK26AUa3U='
	remote = 'socks5'

	# ssh -R 模式的客户端
	# to generate this, run: ./portguard gen-cli -c config.toml -o rclient -s 1 -t 127.0.0.1:8000
	[clients."h6M/DaKv5IOMM4Y2dkiZKpudQ5BCO5DvnNNWqZczGXs="]
	name = 'reverse proxy client'
	pubkey = 'h6M/DaKv5IOMM4Y2dkiZKpudQ5BCO5DvnNNWqZczGXs='
	remote = ['127.0.0.1:8000', 1]

	# ssh -R 访问模式的客户端
	# to generate this, run: ./portguard gen-cli -c config.toml -o rvisitor -s 1
	[clients."Q5VqAyS9dl0CSrOnWOB9XmI0Kb1X83FL6iee3Iio9ls="]
	name = 'reverse proxy visitor'
	pubkey = 'Q5VqAyS9dl0CSrOnWOB9XmI0Kb1X83FL6iee3Iio9ls='
	remote = 1
	```

	除 `ssh -R` 模式的待暴露地址在服务端修改无效之外，其他的所有配置可以在服务端手动更改

3. 服务端运行 `portguard server -c config.toml`

4. 客户端只需直接运行生成的客户端文件，无需任何配置 (如果非要配置，可以配置本地端口和服务端地址，运行 `pgcli -p port -s saddr:sport`).

5. 所有的 TCP 流量都会被加密转发.

## TODO

- [ ] I'm not familar with Noise protocol, now in my code every connection between client and server needs to handshake.
- [x] Set remote address per client.
- [ ] Improve performance.
- [ ] Test.

## 更新日志

### v0.3.0-pre
- 添加 `ssh -R` 功能（只是可以工作，建议使用现有项目，如 frp 或 rathole 来配合 `-L` 模式）
- 需要更多测试

### v0.2.1
- 添加 `x86_64-apple-darwin` 支持（未测试）
- 规范化各平台的数据段的名称
- 任何一个平台的服务端都可以为其他所有平台（windows、linux、macos）生成客户端
- 客户端可以使用 list-key 子命令输出其公钥

### v0.2.0
- 通过内置 SOCKS5 服务，添加 `ssh -D` 功能
- 生成客户端时可以指定输入文件（默认是当前文件）

### v0.1.0
- 基本的 `ssh -L` 功能


## Acknowledgement

本项目的开发离不开以下开源项目：

- [dend.ro's blog article about self-modify binary](https://blog.dend.ro/self-modifying-rust/), I learned how to modify binary.
- [snowstorm](https://github.com/black-binary/snowstorm), I use NoiseStream from this project for convenience
and add some code for timeout when reading from handshake message.
- [fast-socks5](https://github.com/dizda/fast-socks5), I use Socks5Socket from this library as a built-in SOCKS5 server.
- [rust-yamux](https://github.com/libp2p/rust-yamux), I use yamux from this library to impl reverse proxy.