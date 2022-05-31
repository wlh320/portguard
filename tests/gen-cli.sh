./portguard gen-cli -c test.toml -o normal -n normal -t 127.0.0.1:8022
./portguard gen-cli -c test.toml -o socks5 -n socks5 -t socks5
./portguard gen-cli -c test.toml -o rclient -n rclient -s 1 -t 127.0.0.1:8022
./portguard gen-cli -c test.toml -o rvisitor -n rvisitor -s 1
./portguard gen-cli -c test.toml -o rclient_socks5 -n rclient_socks5 -s 2 -t socks5
./portguard gen-cli -c test.toml -o rvisitor_socks5 -n rvisitor_socks5 -s 2
