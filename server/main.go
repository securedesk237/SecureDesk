package main

import (
	"crypto/tls"
	"flag"
	"log"
	"os"
	"os/signal"
	"syscall"

	"github.com/securedesk/relay/relay"
)

func main() {
	// Command line flags
	listenAddr := flag.String("listen", ":8443", "Listen address for TLS connections")
	certFile := flag.String("cert", "certs/server.crt", "TLS certificate file")
	keyFile := flag.String("key", "certs/server.key", "TLS private key file")
	flag.Parse()

	// Load TLS certificate
	cert, err := tls.LoadX509KeyPair(*certFile, *keyFile)
	if err != nil {
		log.Fatalf("Failed to load TLS certificate: %v", err)
	}

	// TLS 1.3 only configuration
	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
		MinVersion:   tls.VersionTLS13,
		MaxVersion:   tls.VersionTLS13,
		CipherSuites: []uint16{
			tls.TLS_CHACHA20_POLY1305_SHA256,
			tls.TLS_AES_256_GCM_SHA384,
		},
	}

	// Create and start relay server
	server := relay.NewServer(tlsConfig)

	go func() {
		log.Printf("SecureDesk Relay starting on %s", *listenAddr)
		if err := server.ListenAndServe(*listenAddr); err != nil {
			log.Fatalf("Server error: %v", err)
		}
	}()

	// Wait for shutdown signal
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
	<-sigChan

	log.Println("Shutting down relay server...")
	server.Shutdown()
}
