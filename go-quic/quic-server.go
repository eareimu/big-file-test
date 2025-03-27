package main

import (
	"context"
	"fmt"
	"log"
	"net"
	"net/http"
	"time"

	"github.com/quic-go/quic-go"
	"github.com/quic-go/quic-go/http3"
	"github.com/quic-go/quic-go/logging"
)

// 日志中间件
func loggingMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := time.Now()

		// 记录请求开始
		log.Printf("新请求: %s %s %s\n", r.RemoteAddr, r.Method, r.URL.Path)
		log.Printf("请求头: %v\n", r.Header)

		// 包装 ResponseWriter 以捕获状态码
		wrapper := &responseWrapper{
			ResponseWriter: w,
			statusCode:     http.StatusOK,
		}

		next.ServeHTTP(wrapper, r)

		// 记录请求完成
		duration := time.Since(start)
		log.Printf("请求完成: %s %s %s - 状态码: %d - 耗时: %v\n",
			r.RemoteAddr, r.Method, r.URL.Path, wrapper.statusCode, duration)
	})
}

// ResponseWriter 包装器
type responseWrapper struct {
	http.ResponseWriter
	statusCode int
}

func (rw *responseWrapper) WriteHeader(code int) {
	rw.statusCode = code
	rw.ResponseWriter.WriteHeader(code)
}

func main() {
	// 设置日志格式
	log.SetFlags(log.Ldate | log.Ltime | log.Lmicroseconds)

	mux := http.NewServeMux()

	// 创建文件服务器，使用 "../" 作为根目录
	fileServer := http.FileServer(http.Dir("../"))
	// 添加日志中间件
	mux.Handle("/", loggingMiddleware(fileServer))

	fmt.Println("Starting HTTP/3 server on [::1]:4430")
	fmt.Println("Serving files from ../")

	// 创建 QUIC 传输配置
	quicConfig := &http3.Server{
		Addr:    "[::1]:4430", // 设置监听地址
		Handler: mux,
		QUICConfig: &quic.Config{
			Tracer: func(ctx context.Context, p logging.Perspective, connectionID quic.ConnectionID) *logging.ConnectionTracer {
				return &logging.ConnectionTracer{
					StartedConnection: func(local, remote net.Addr, srcConnID, destConnID logging.ConnectionID) {
						log.Printf("新连接建立: %s -> %s\n", remote, local)
					},
					ClosedConnection: func(err error) {
						if err != nil {
							log.Printf("连接关闭，错误: %v\n", err)
						} else {
							log.Printf("连接正常关闭\n")
						}
					},
				}
			},
		},
	}

	err := quicConfig.ListenAndServeTLS("../server.crt", "../server.key")
	if err != nil {
		log.Fatal("HTTP/3 server error:", err)
	}
}
