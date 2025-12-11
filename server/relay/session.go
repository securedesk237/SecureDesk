package relay

import (
	"crypto/rand"
	"encoding/hex"
	"sync"
)

// Session represents an active remote session between technician and endpoint
// The relay has NO knowledge of session content - it only forwards encrypted frames
type Session struct {
	ID         string
	Technician *Client
	Endpoint   *Client
	done       chan struct{}
	closeOnce  sync.Once
}

// NewSession creates a new session
func NewSession(id string, technician, endpoint *Client) *Session {
	return &Session{
		ID:         id,
		Technician: technician,
		Endpoint:   endpoint,
		done:       make(chan struct{}),
	}
}

// Close terminates the session
func (s *Session) Close() {
	s.closeOnce.Do(func() {
		close(s.done)
		s.Technician.Close()
		// Don't close endpoint - it may accept other sessions
	})
}

// Done returns a channel that's closed when the session ends
func (s *Session) Done() <-chan struct{} {
	return s.done
}

// IsClosed returns true if the session is closed
func (s *Session) IsClosed() bool {
	select {
	case <-s.done:
		return true
	default:
		return false
	}
}

// SessionManager manages active sessions and registered endpoints
// NOTE: No logging, no persistence, no history - privacy by design
type SessionManager struct {
	endpoints map[string]*Client  // key: public key hash
	sessions  map[string]*Session // key: session ID
	mu        sync.RWMutex
}

// NewSessionManager creates a new session manager
func NewSessionManager() *SessionManager {
	return &SessionManager{
		endpoints: make(map[string]*Client),
		sessions:  make(map[string]*Session),
	}
}

// RegisterEndpoint registers an endpoint by its public key hash
func (sm *SessionManager) RegisterEndpoint(client *Client) {
	sm.mu.Lock()
	defer sm.mu.Unlock()

	// Close existing endpoint with same ID if any
	if existing, ok := sm.endpoints[client.ID]; ok {
		existing.Close()
	}
	sm.endpoints[client.ID] = client
}

// UnregisterEndpoint removes an endpoint
func (sm *SessionManager) UnregisterEndpoint(id string) {
	sm.mu.Lock()
	defer sm.mu.Unlock()
	delete(sm.endpoints, id)
}

// GetEndpoint retrieves an endpoint by ID
func (sm *SessionManager) GetEndpoint(id string) *Client {
	sm.mu.RLock()
	defer sm.mu.RUnlock()
	return sm.endpoints[id]
}

// EndpointCount returns the number of registered endpoints
func (sm *SessionManager) EndpointCount() int {
	sm.mu.RLock()
	defer sm.mu.RUnlock()
	return len(sm.endpoints)
}

// SessionCount returns the number of active sessions
func (sm *SessionManager) SessionCount() int {
	sm.mu.RLock()
	defer sm.mu.RUnlock()
	return len(sm.sessions)
}

// CreateSession creates a new session between technician and endpoint
func (sm *SessionManager) CreateSession(technician, endpoint *Client) *Session {
	sm.mu.Lock()
	defer sm.mu.Unlock()

	// Generate random session ID (not logged anywhere)
	idBytes := make([]byte, 16)
	rand.Read(idBytes)
	sessionID := hex.EncodeToString(idBytes)

	session := NewSession(sessionID, technician, endpoint)

	// Pair the clients for bidirectional forwarding
	technician.Paired = endpoint
	endpoint.Paired = technician

	sm.sessions[sessionID] = session
	return session
}

// CloseSession closes and removes a session
func (sm *SessionManager) CloseSession(id string) {
	sm.mu.Lock()
	session, exists := sm.sessions[id]
	if exists {
		delete(sm.sessions, id)
	}
	sm.mu.Unlock()

	if session != nil {
		// Unpair clients
		if session.Technician != nil {
			session.Technician.Paired = nil
		}
		if session.Endpoint != nil {
			session.Endpoint.Paired = nil
		}
		session.Close()
	}
}

// CloseAll closes all sessions and endpoints
func (sm *SessionManager) CloseAll() {
	sm.mu.Lock()
	defer sm.mu.Unlock()

	for _, session := range sm.sessions {
		session.Close()
	}
	for _, endpoint := range sm.endpoints {
		endpoint.Close()
	}

	sm.sessions = make(map[string]*Session)
	sm.endpoints = make(map[string]*Client)
}
