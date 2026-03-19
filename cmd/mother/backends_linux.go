//go:build linux

package main

import (
	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/backend/wayland"
	"github.com/kickthemoon0817/mother-terminal/internal/backend/x11"
)

func registerPlatformBackends(reg *backend.Registry) {
	reg.Register(&x11.Backend{})
	reg.Register(&wayland.Backend{})
}
