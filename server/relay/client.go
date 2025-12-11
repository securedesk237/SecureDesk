package relay

import (
	"bufio"
	"crypto/tls"
	"encoding/binary"
	"errors"
	"io"
	"net"
	"sync"
	"time"
)

// Client types
const (
	ClientTypeEndpoint   uint8 = 0x01
	ClientTypeTechnician uint8 = 0x02
)

// Errors
var (
	ErrEndpointNotFound = errors.New("endpoint not found")
	ErrInvalidHandshake = errors.New("invalid handshake")
	ErrSessionClosed    = errors.New("session closed")
	ErrFrameTooLarge    = errors.New("frame too large")
)

// Frame represents a protocol frame
// The relay does NOT decrypt or inspect the payload
type Frame struct {
	ChannelID uint8
	Payload   []byte
}

// Client represents a connected client (endpoint or technician)
type Client struct {
	conn          net.Conn
	reader        *bufio.Reader
	writer        *bufio.Writer
	Type          uint8
	ID            string
	TargetID      string
	PublicKeyHash string
	Paired        *Client
	Done          chan struct{}
	writeMu       sync.Mutex
	bufferPool    *sync.Pool
	closeOnce     sync.Once
}

// NewClient creates a new client wrapper with buffered I/O
func NewClient(conn *tls.Conn, pool *sync.Pool) *Client {
	// Use larger buffers for high throughput
	return &Client{
		conn:       conn,
		reader:     bufio.NewReaderSize(conn, 128*1024), // 128KB read buffer
		writer:     bufio.NewWriterSize(conn, 128*1024), // 128KB write buffer
		Done:       make(chan struct{}),
		bufferPool: pool,
	}
}

// ReadHandshake reads the initial handshake from client
func (c *Client) ReadHandshake() error {
	// Set read deadline for handshake
	c.conn.SetReadDeadline(time.Now().Add(10 * time.Second))
	defer c.conn.SetReadDeadline(time.Time{})

	// Read client type (1 byte)
	typeBuf := make([]byte, 1)
	if _, err := io.ReadFull(c.reader, typeBuf); err != nil {
		return err
	}
	c.Type = typeBuf[0]

	// Read ID length (2 bytes) and ID
	lenBuf := make([]byte, 2)
	if _, err := io.ReadFull(c.reader, lenBuf); err != nil {
		return err
	}
	idLen := binary.BigEndian.Uint16(lenBuf)

	if idLen > 1024 {
		return ErrInvalidHandshake
	}

	idBuf := make([]byte, idLen)
	if _, err := io.ReadFull(c.reader, idBuf); err != nil {
		return err
	}
	c.ID = string(idBuf)
	c.PublicKeyHash = c.ID

	// For technicians, also read target endpoint ID
	if c.Type == ClientTypeTechnician {
		if _, err := io.ReadFull(c.reader, lenBuf); err != nil {
			return err
		}
		targetLen := binary.BigEndian.Uint16(lenBuf)

		if targetLen > 1024 {
			return ErrInvalidHandshake
		}

		targetBuf := make([]byte, targetLen)
		if _, err := io.ReadFull(c.reader, targetBuf); err != nil {
			return err
		}
		c.TargetID = string(targetBuf)
	}

	return nil
}

// ReadFrame reads a protocol frame from the connection
// Frame format: [channel_id (1 byte)][length (3 bytes)][payload]
func (c *Client) ReadFrame() (*Frame, error) {
	header := make([]byte, 4)
	if _, err := io.ReadFull(c.reader, header); err != nil {
		return nil, err
	}

	channelID := header[0]
	length := uint32(header[1])<<16 | uint32(header[2])<<8 | uint32(header[3])

	// Sanity check: max 16MB per frame
	if length > 16*1024*1024 {
		return nil, ErrFrameTooLarge
	}

	// Use pooled buffer if available and appropriate size
	var payload []byte
	if c.bufferPool != nil && length <= 64*1024 {
		bufPtr := c.bufferPool.Get().(*[]byte)
		payload = (*bufPtr)[:length]
		defer c.bufferPool.Put(bufPtr)

		if _, err := io.ReadFull(c.reader, payload); err != nil {
			return nil, err
		}

		// Copy to new slice since we're returning the buffer to pool
		payloadCopy := make([]byte, length)
		copy(payloadCopy, payload)
		payload = payloadCopy
	} else {
		payload = make([]byte, length)
		if _, err := io.ReadFull(c.reader, payload); err != nil {
			return nil, err
		}
	}

	return &Frame{
		ChannelID: channelID,
		Payload:   payload,
	}, nil
}

// WriteFrame writes a protocol frame to the connection
// Uses buffered writes and combines header+payload for efficiency
func (c *Client) WriteFrame(frame *Frame) error {
	c.writeMu.Lock()
	defer c.writeMu.Unlock()

	// Build header
	header := [4]byte{
		frame.ChannelID,
		byte(len(frame.Payload) >> 16),
		byte(len(frame.Payload) >> 8),
		byte(len(frame.Payload)),
	}

	// Write header and payload together
	if _, err := c.writer.Write(header[:]); err != nil {
		return err
	}
	if _, err := c.writer.Write(frame.Payload); err != nil {
		return err
	}

	// Flush immediately for low latency
	return c.writer.Flush()
}

// SendError sends an error message to the client
func (c *Client) SendError(err error) {
	errMsg := []byte(err.Error())
	frame := &Frame{
		ChannelID: 0x00, // Control channel
		Payload:   append([]byte{0xFF}, errMsg...), // 0xFF = error type
	}
	c.WriteFrame(frame)
}

// SendSuccess sends a success response to the client
func (c *Client) SendSuccess() {
	frame := &Frame{
		ChannelID: 0x00, // Control channel
		Payload:   []byte{0x01}, // 0x01 = success/session established
	}
	c.WriteFrame(frame)
}

// NotifyConnection notifies endpoint of incoming technician connection
func (c *Client) NotifyConnection(technicianKeyHash string) {
	frame := &Frame{
		ChannelID: 0x00,
		Payload:   append([]byte{0x02}, []byte(technicianKeyHash)...), // 0x02 = session request
	}
	c.WriteFrame(frame)
}

// Close closes the client connection
func (c *Client) Close() {
	c.closeOnce.Do(func() {
		close(c.Done)
		c.conn.Close()
	})
}

// SetDeadline sets read/write deadline
func (c *Client) SetDeadline(t time.Time) error {
	return c.conn.SetDeadline(t)
}

// RemoteAddr returns the remote address
func (c *Client) RemoteAddr() net.Addr {
	return c.conn.RemoteAddr()
}
