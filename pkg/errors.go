package pkg

import "errors"

var (
	// ErrSessionNotFound is returned when a session cannot be found.
	ErrSessionNotFound = errors.New("session not found")

	// ErrBackendUnavailable is returned when the requested backend is not available.
	ErrBackendUnavailable = errors.New("backend unavailable on this platform")

	// ErrSendKeysFailed is returned when keystroke injection fails.
	ErrSendKeysFailed = errors.New("failed to send keys to session")

	// ErrTimeout is returned when an operation times out.
	ErrTimeout = errors.New("operation timed out")

	// ErrInvalidTransition is returned for invalid session state transitions.
	ErrInvalidTransition = errors.New("invalid state transition")

	// ErrSessionAlreadyExists is returned when adding a duplicate session.
	ErrSessionAlreadyExists = errors.New("session already exists")

	// ErrConfigInvalid is returned when the config file is invalid.
	ErrConfigInvalid = errors.New("invalid configuration")

	// ErrReadOutputFailed is returned when reading output from a session fails.
	ErrReadOutputFailed = errors.New("failed to read output from session")
)
