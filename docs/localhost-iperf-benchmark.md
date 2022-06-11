# localhost iperf benchmark

## Environment

### Hardware

- CPU: AMD Ryzen 7 PRO 2700U w/ Radeon Vega Mobile Gfx (8) @ 2.200GHz 

### Software

- OS: Arch Linux x86_64 
- iperf 3.11 (cJSON 1.7.13)
- proxychains 4.16-1
- OpenSSH_9.0p1, OpenSSL 1.1.1o  3 May 2022
- frp 0.43.0
- portguard 0.3.2

## Steps

1. run iperf3 server: `iperf3 -s`
2. forward port `5201` to `6000`
3. run iperf3 client:
    - `-D` mode: `proxychains iperf3 -c 127.0.0.1 -p 5201`
    - others: `iperf3 -c 127.0.0.1 -p 6000`

We compare `portguard` with `ssh` and `frp` in different modes.
Project with the most similar behavior as `portguard in -R mode` is `frp with stcp and encryption`.

## Results

### without port forwarding

```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec  27.5 GBytes  23.6 Gbits/sec    2             sender
[  5]   0.00-10.00  sec  27.5 GBytes  23.6 Gbits/sec                  receiver
```

### -L mode

- ssh: `ssh -L 6000:127.0.0.1:5201 wlh@127.0.0.1`
```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec   350 MBytes   293 Mbits/sec    0             sender
[  5]   0.00-10.00  sec   344 MBytes   289 Mbits/sec                  receiver
```

- portguard: use config in `tests/gen-cli.sh`
    - server: `./portguard server -c test.toml`
    - client: `./normal.exe 6000`
```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec  2.39 GBytes  2.05 Gbits/sec    0             sender
[  5]   0.00-10.00  sec  2.37 GBytes  2.04 Gbits/sec                  receiver
```

### -D mode

- ssh: `ssh -D 6000 wlh@127.0.0.1`

```
[ ID] Interval           Transfer     Bitrate         Retr
[  9]   0.00-10.00  sec   337 MBytes   283 Mbits/sec    0             sender
[  9]   0.00-10.00  sec   331 MBytes   278 Mbits/sec                  receiver
```

- portguard: use config in `tests/gen-cli.sh`
    - server: `./portguard server -c test.toml`
    - client: `./socks5.exe 6000`
 
```
[ ID] Interval           Transfer     Bitrate         Retr
[  9]   0.00-10.00  sec  2.38 GBytes  2.04 Gbits/sec    0             sender
[  9]   0.00-10.00  sec  2.36 GBytes  2.03 Gbits/sec                  receiver
```

### -R mode

- ssh: `ssh -R 6000:127.0.0.1:5201 wlh@127.0.0.1`
```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec   330 MBytes   277 Mbits/sec    0             sender
[  5]   0.00-10.00  sec   325 MBytes   273 Mbits/sec                  receiver
```

- frp in stcp mode with encryption:
    - server: 
    ```ini
    # frps.ini
    [common]
    bind_port = 8848
    ```
    - client: stcp mode with encryption
    ```ini
    # frpc.ini
    [common]
    server_addr = 127.0.0.1
    server_port = 8848

    [iperf]
    type = stcp
    sk = abcdefg
    local_ip = 127.0.0.1
    local_port = 5201
    use_encryption = true
    ```
    - visitor: stcp mode with encryption
    ```ini
    # visitor.ini
    [common]
    server_addr = 127.0.0.1
    server_port = 8848

    [iperf_visitor]
    type = stcp
    role = visitor
    server_name = iperf
    sk = abcdefg
    bind_addr = 127.0.0.1
    bind_port = 6000
    use_encryption = true
    ```
 
```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec   906 MBytes   760 Mbits/sec    0             sender
[  5]   0.00-10.00  sec   898 MBytes   754 Mbits/sec                  receiver
```

- frp in stcp mode without encryption:
```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec  2.04 GBytes  1.75 Gbits/sec    0             sender
[  5]   0.00-10.00  sec  2.03 GBytes  1.75 Gbits/sec                  receiver
```

- portguard: use config in `tests/gen-cli.sh`
    - server: `./portguard server -c test.toml`
    - client: `./rclient.exe`
    - visitor: `./rvisitor.exe 6000`
 
```
[ ID] Interval           Transfer     Bitrate         Retr
[  5]   0.00-10.00  sec  1.30 GBytes  1.12 Gbits/sec    0             sender
[  5]   0.00-10.00  sec  1.29 GBytes  1.11 Gbits/sec                  receiver
```

## Conclusion

It shows the performance of port forwarding in the environment with unlimited network bandwidth, the bottleneck is CPU.