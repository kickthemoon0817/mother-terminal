package backend

import "github.com/kickthemoon0817/mother-terminal/pkg"

// Injector is the interface that all injection backends must implement.
type Injector interface {
	// Discover finds running AI CLI sessions managed by this backend.
	Discover() ([]pkg.Session, error)

	// SendKeys injects a string followed by Enter into the target session.
	SendKeys(session pkg.Session, text string) error

	// ReadOutput captures recent output lines from the session.
	ReadOutput(session pkg.Session, lines int) (string, error)

	// Ping checks if the session is alive and responsive.
	Ping(session pkg.Session) (pkg.PingResult, error)

	// IsAvailable reports whether this backend works on the current OS/environment.
	IsAvailable() bool

	// Name returns the backend type identifier.
	Name() pkg.BackendType
}
