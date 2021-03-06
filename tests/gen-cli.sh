#!/bin/sh
cp ../target/x86_64-unknown-linux-musl/release/portguard .
cp ../target/x86_64-unknown-linux-musl/release/pgcli .
cp test.toml test.toml.origin
# all types of client
./portguard gen-cli -c test.toml -i ./pgcli -o normal.exe -n normal -t 127.0.0.1:5201
./portguard gen-cli -c test.toml -i ./pgcli -o socks5.exe -n socks5 -t socks5 --password
./portguard gen-cli -c test.toml -i ./pgcli -o rclient.exe -n rclient -s 1 -t 127.0.0.1:5201
./portguard gen-cli -c test.toml -i ./pgcli -o rvisitor.exe -n rvisitor -s 1
./portguard gen-cli -c test.toml -i ./pgcli -o rclient_socks5.exe -n rclient_socks5 -s 2 -t socks5
./portguard gen-cli -c test.toml -i ./pgcli -o rvisitor_socks5.exe -n rvisitor_socks5 -s 2
./portguard server -c test.toml
