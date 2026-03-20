package main

import (
	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/backend/tmux"
	"github.com/kickthemoon0817/mother-terminal/internal/pty"
)

func registerCommonBackends(reg *backend.Registry) *pty.Backend {
	reg.Register(&tmux.Backend{})
	pb := pty.NewBackend()
	reg.Register(pb)
	return pb
}
