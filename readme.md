生成测试文件，参数为文件大小（MB），产物为`rand-file-<size>M`

``` shell
./gen-file.sh 32
```

启动server。不指定qlog参数即禁用qlog
``` shell
cargo run --release --bin=server -- --qlog-dir=qlog --bind=[::]:35467 
```

启动client。不指定qlog参数即禁用qlog
``` shell
cargo run --release --bin=client -- --qlog-dir=qlog --server=[::1]:35467 --file=rand-file-32M
```