# SecureDesk Wire Protocol Specification

## Frame Format

All communication uses a simple binary framing protocol:

```
+-------------+----------------+----------------------+
| channel_id  |     length     |   ciphertext payload |
|  (1 byte)   |   (3 bytes)    |    (variable)        |
+-------------+----------------+----------------------+
```

## Channel IDs

| ID   | Channel         | Description                           |
|------|-----------------|---------------------------------------|
| 0x00 | Control         | Session control, handshake, keepalive |
| 0x01 | Video           | Screen capture frames                 |
| 0x02 | Input           | Keyboard and mouse events             |
| 0x03 | Clipboard       | Clipboard data transfer               |
| 0x04 | File            | File transfer (optional)              |
| 0x05 | Privacy         | Privacy mode control                  |

## Message Types

### Control Channel (0x00)

| Type | Name            | Direction      | Description                    |
|------|-----------------|----------------|--------------------------------|
| 0x01 | Handshake       | Both           | Noise protocol handshake       |
| 0x02 | SessionStart    | Tech -> End    | Request session start          |
| 0x03 | SessionAccept   | End -> Tech    | Accept session                 |
| 0x04 | SessionEnd      | Both           | End session                    |
| 0x05 | Keepalive       | Both           | Connection keepalive           |
| 0x06 | PolicyUpdate    | Relay -> Both  | Policy configuration           |

### Video Channel (0x01)

| Type | Name            | Direction      | Description                    |
|------|-----------------|----------------|--------------------------------|
| 0x01 | FrameKey        | End -> Tech    | Keyframe (full image)          |
| 0x02 | FrameDelta      | End -> Tech    | Delta frame (changes only)     |
| 0x03 | FrameRequest    | Tech -> End    | Request keyframe               |
| 0x04 | QualitySet      | Tech -> End    | Set quality/resolution         |

### Input Channel (0x02)

| Type | Name            | Direction      | Description                    |
|------|-----------------|----------------|--------------------------------|
| 0x01 | MouseMove       | Tech -> End    | Mouse position update          |
| 0x02 | MouseButton     | Tech -> End    | Mouse button event             |
| 0x03 | MouseScroll     | Tech -> End    | Mouse scroll event             |
| 0x04 | KeyDown         | Tech -> End    | Key press                      |
| 0x05 | KeyUp           | Tech -> End    | Key release                    |

### Privacy Channel (0x05)

| Type | Name            | Direction      | Description                    |
|------|-----------------|----------------|--------------------------------|
| 0x01 | BlackScreenOn   | Tech -> End    | Enable black screen            |
| 0x02 | BlackScreenOff  | Tech -> End    | Disable black screen           |
| 0x03 | InputBlockOn    | Tech -> End    | Block local input              |
| 0x04 | InputBlockOff   | Tech -> End    | Unblock local input            |
| 0x05 | StatusAck       | End -> Tech    | Acknowledge privacy change     |

## Encryption

All payloads are encrypted using ChaCha20-Poly1305 with the session key derived from the Noise handshake.

Frame encryption format:
```
+-------------+----------------------+----------+
|   nonce     |     ciphertext       |   tag    |
| (12 bytes)  |     (variable)       | (16 bytes)|
+-------------+----------------------+----------+
```

## Noise Protocol Pattern

SecureDesk uses Noise_XK pattern:
- Initiator (Technician) knows responder's (Endpoint) static public key
- Provides mutual authentication and forward secrecy
