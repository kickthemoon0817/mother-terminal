package session

import (
	"fmt"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

// Valid state transitions.
var validTransitions = map[pkg.SessionStatus][]pkg.SessionStatus{
	pkg.StatusDiscovered: {pkg.StatusActive, pkg.StatusDead},
	pkg.StatusActive:     {pkg.StatusStalled, pkg.StatusDead},
	pkg.StatusStalled:    {pkg.StatusActive, pkg.StatusDead},
	pkg.StatusDead:       {}, // terminal state
}

// ValidateTransition checks if a state transition is allowed.
func ValidateTransition(from, to pkg.SessionStatus) error {
	allowed, ok := validTransitions[from]
	if !ok {
		return fmt.Errorf("%w: unknown state %q", pkg.ErrInvalidTransition, from)
	}
	for _, s := range allowed {
		if s == to {
			return nil
		}
	}
	return fmt.Errorf("%w: %s -> %s", pkg.ErrInvalidTransition, from, to)
}
