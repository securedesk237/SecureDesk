package relay

import (
	"crypto/tls"
	"log"
	"net"
	"runtime"
	"sync"
	"sync/atomic"
	"time"
)

// Server is the SecureDesk relay server
// Optimized for low-latency, high-throughput streaming
// Does NOT log, store, or inspect any payload data
type Server struct {
	tlsConfig    *tls.Config
	listener     net.Listener
	sessions     *SessionManager
	shutdown     chan struct{}
	wg           sync.WaitGroup
	activeConns  int64
	bufferPool   *sync.Pool
}

// NewServer creates a new high-performance relay server
func NewServer(tlsConfig *tls.Config) *Server {
	return &Server{
		tlsConfig: tlsConfig,
		sessions:  NewSessionManager(),
		shutdown:  make(chan struct{}),
		bufferPool: &sync.Pool{
			New: func() interface{} {
				// Pre-allocate 64KB buffers for frame reading
				buf := make([]byte, 64*1024)
				return &buf
			},
		},
	}
}

// ListenAndServe starts the relay server with optimized settings
func (s *Server) ListenAndServe(addr string) error {
	// Create TCP listener first for socket options
	tcpAddr, err := net.ResolveTCPAddr("tcp", addr)
	if err != nil {
		return err
	}

	tcpListener, err := net.ListenTCP("tcp", tcpAddr)
	if err != nil {
		return err
	}

	// Wrap with TLS
	s.listener = tls.NewListener(tcpListener, s.tlsConfig)

	// Use multiple accept goroutines for high connection rates
	numAcceptors := runtime.NumCPU()
	if numAcceptors < 2 {
		numAcceptors = 2
	}

	for i := 0; i < numAcceptors; i++ {
		s.wg.Add(1)
		go s.acceptLoop(tcpListener)
	}

	// Wait for shutdown
	<-s.shutdown
	return nil
}

// acceptLoop handles incoming connections
func (s *Server) acceptLoop(tcpListener *net.TCPListener) {
	defer s.wg.Done()

	for {
		select {
		case <-s.shutdown:
			return
		default:
		}

		// Short deadline for responsive shutdown
		tcpListener.SetDeadline(time.Now().Add(500 * time.Millisecond))

		conn, err := s.listener.Accept()
		if err != nil {
			if netErr, ok := err.(net.Error); ok && netErr.Timeout() {
				continue
			}
			select {
			case <-s.shutdown:
				return
			default:
				continue
			}
		}

		atomic.AddInt64(&s.activeConns, 1)
		go s.handleConnection(conn)
	}
}

// Shutdown gracefully stops the server
func (s *Server) Shutdown() {
	close(s.shutdown)
	if s.listener != nil {
		s.listener.Close()
	}
	s.wg.Wait()
	s.sessions.CloseAll()
	log.Printf("Relay shutdown complete. Peak connections: %d", atomic.LoadInt64(&s.activeConns))
}

// handleConnection processes a new client connection with optimizations
func (s *Server) handleConnection(conn net.Conn) {
	// NOTE: We deliberately do NOT log IP addresses or client IDs for privacy
	defer func() {
		atomic.AddInt64(&s.activeConns, -1)
		conn.Close()
	}()

	tlsConn, ok := conn.(*tls.Conn)
	if !ok {
		log.Printf("Not a TLS connection")
		return
	}

	// Optimize TCP settings for low latency
	if tcpConn, ok := conn.(*net.TCPConn); ok {
		tcpConn.SetNoDelay(true)       // Disable Nagle's algorithm
		tcpConn.SetKeepAlive(true)     // Enable keepalive
		tcpConn.SetKeepAlivePeriod(30 * time.Second)
		tcpConn.SetReadBuffer(256 * 1024)  // 256KB read buffer
		tcpConn.SetWriteBuffer(256 * 1024) // 256KB write buffer
	}

	// Set handshake timeout
	tlsConn.SetDeadline(time.Now().Add(10 * time.Second))

	// Complete TLS handshake
	if err := tlsConn.Handshake(); err != nil {
		return
	}

	// Clear deadline after handshake
	tlsConn.SetDeadline(time.Time{})

	// Read client type and session info
	client := NewClient(tlsConn, s.bufferPool)
	if err := client.ReadHandshake(); err != nil {
		return
	}

	// Register client based on type (no logging of IDs for privacy)
	switch client.Type {
	case ClientTypeEndpoint:
		s.handleEndpoint(client)
	case ClientTypeTechnician:
		s.handleTechnician(client)
	}
}

// handleEndpoint manages an endpoint connection
func (s *Server) handleEndpoint(client *Client) {
	// Register endpoint with its ID
	s.sessions.RegisterEndpoint(client)
	defer s.sessions.UnregisterEndpoint(client.ID)

	// Wait for session or disconnect
	<-client.Done
}

// handleTechnician manages a technician connection
func (s *Server) handleTechnician(client *Client) {
	targetID := client.TargetID

	// Find the endpoint (no logging of IDs for privacy)
	endpoint := s.sessions.GetEndpoint(targetID)
	if endpoint == nil {
		client.SendError(ErrEndpointNotFound)
		return
	}

	// Create session pairing
	session := s.sessions.CreateSession(client, endpoint)
	defer s.sessions.CloseSession(session.ID)

	// Send success response to technician
	client.SendSuccess()

	// Notify endpoint of incoming connection
	endpoint.NotifyConnection(client.PublicKeyHash)

	// Bridge traffic with zero-copy forwarding
	s.bridgeSession(session)
}

// bridgeSession forwards traffic bidirectionally with minimal overhead
func (s *Server) bridgeSession(session *Session) {
	var wg sync.WaitGroup
	wg.Add(2)

	// Technician -> Endpoint (high priority for input)
	go func() {
		defer wg.Done()
		s.copyFrames(session.Technician, session.Endpoint, session)
	}()

	// Endpoint -> Technician (high throughput for video)
	go func() {
		defer wg.Done()
		s.copyFrames(session.Endpoint, session.Technician, session)
	}()

	wg.Wait()
}

// copyFrames copies frames from src to dst with pooled buffers
func (s *Server) copyFrames(src, dst *Client, session *Session) {
	for {
		select {
		case <-session.done:
			return
		default:
		}

		frame, err := src.ReadFrame()
		if err != nil {
			session.Close()
			return
		}

		if err := dst.WriteFrame(frame); err != nil {
			session.Close()
			return
		}
	}
}

// ActiveConnections returns current connection count
func (s *Server) ActiveConnections() int64 {
	return atomic.LoadInt64(&s.activeConns)
}
