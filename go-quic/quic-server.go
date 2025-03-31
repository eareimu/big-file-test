package main

import (
	"fmt"
	"log"
	"net/http"

	"github.com/jessevdk/go-flags"
	"github.com/quic-go/quic-go/http3"
)

func main() {
	// 设置日志格式
	log.SetFlags(log.Ldate | log.Ltime | log.Lmicroseconds)

	var opts struct {
		Address  string `short:"a" long:"address" description:"Listening address" required:"true"`
		CertFile string `short:"c" long:"cert" description:"Path to the certificate file" required:"true"`
		KeyFile  string `short:"k" long:"key" description:"Path to the key file" required:"true"`
	}

	_, err := flags.Parse(&opts)
	if err != nil {
		log.Fatal("Failed to parse arguments:", err)
	}
	mux := http.NewServeMux()

	// 创建文件服务器，使用 "." 作为根目录
	fileServer := http.FileServer(http.Dir("."))
	mux.Handle("/", fileServer)

	fmt.Println("Starting HTTP/3 server on ", opts.Address)
	fmt.Println("Serving files from ./")

	err = http3.ListenAndServeTLS(
		opts.Address,  // 从命令行获取监听地址
		opts.CertFile, // 从命令行获取证书路径
		opts.KeyFile,  // 从命令行获取密钥路径
		mux,
	)

	if err != nil {
		log.Fatal("HTTP/3 server error:", err)
	}
}
