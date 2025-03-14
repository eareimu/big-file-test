生成测试文件，参数为文件大小（MB），产物为`rand-file-<size>M`

### 运行

``` shell
./gen-file.sh 32
```

启动server。不指定qlog参数即禁用qlog。所有参数都是可选的，server会打印它listen了哪个地址
``` shell
cargo run --release --bin=server -- --qlog-dir=qlog --bind=[::]:35467 
```

启动client。不指定qlog参数即禁用qlog。file和server参数必选
``` shell
cargo run --release --bin=client -- --qlog-dir=qlog --server=[::1]:35467 --file=rand-file-32M
```

### 调试

有两脚本可以根据输出分析send_waker和burst，位于`scripts`目录下。

使用instrument分析时如遇到签名错误，使用`debug.plist`自签：
``` shell 
codesign -s - -v -f --entitlements debug.plist target/release/client
```