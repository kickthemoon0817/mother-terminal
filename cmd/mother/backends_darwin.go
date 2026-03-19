//go:build darwin

package main

import (
	"github.com/kickthemoon0817/mother-terminal/internal/backend"
	"github.com/kickthemoon0817/mother-terminal/internal/backend/macos"
)

func registerPlatformBackends(reg *backend.Registry) {
	reg.Register(&macos.Backend{})
}
