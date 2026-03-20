package remote

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"
	"time"

	"github.com/kickthemoon0817/mother-terminal/pkg"
)

var validAddress = regexp.MustCompile(`^[a-zA-Z0-9._@%\-]+$`)

// Host represents a remote machine connected via SSH (e.g., over Tailscale).
type Host struct {
	Name    string // friendly name
	Address string // user@host or Tailscale hostname
}

// Client manages remote SSH connections for AI CLI session management.
type Client struct {
	hosts map[string]*Host
}

// NewClient creates a new remote client.
func NewClient() *Client {
	return &Client{
		hosts: make(map[string]*Host),
	}
}

// AddHost registers a remote host.
func (c *Client) AddHost(name, address string) error {
	if !validAddress.MatchString(address) {
		return fmt.Errorf("invalid host address %q", address)
	}
	c.hosts[name] = &Host{Name: name, Address: address}
	return nil
}

// RemoveHost removes a remote host.
func (c *Client) RemoveHost(name string) {
	delete(c.hosts, name)
}

// ListHosts returns all registered hosts.
func (c *Client) ListHosts() []*Host {
	var hosts []*Host
	for _, h := range c.hosts {
		hosts = append(hosts, h)
	}
	return hosts
}

// Ping checks if a remote host is reachable via SSH.
func (c *Client) Ping(name string) (bool, time.Duration, error) {
	host, ok := c.hosts[name]
	if !ok {
		return false, 0, fmt.Errorf("unknown host %q", name)
	}

	start := time.Now()
	cmd := exec.Command("ssh", "-o", "ConnectTimeout=5", "-o", "BatchMode=yes", host.Address, "echo ok")
	out, err := cmd.Output()
	latency := time.Since(start)

	if err != nil {
		return false, latency, nil
	}
	return strings.TrimSpace(string(out)) == "ok", latency, nil
}

// Spawn starts an AI CLI session on a remote host inside tmux.
func (c *Client) Spawn(hostName, cliName string) (*pkg.Session, error) {
	host, ok := c.hosts[hostName]
	if !ok {
		return nil, fmt.Errorf("unknown host %q", hostName)
	}

	sessionName := fmt.Sprintf("mtt-%s-%d", cliName, time.Now().Unix())

	// Create tmux session on remote host
	cmd := exec.Command("ssh", host.Address, "tmux", "new-session", "-d", "-s", sessionName, cliName)
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("failed to spawn %s on %s: %v", cliName, host.Address, err)
	}

	return &pkg.Session{
		ID:        fmt.Sprintf("remote-%s-%s-%s", hostName, cliName, sessionName),
		Name:      fmt.Sprintf("%s@%s [%s]", cliName, hostName, sessionName),
		CLI:       pkg.CLIType(cliName),
		Backend:   pkg.BackendTmux,
		Target:    sessionName + ":0.0",
		Status:    pkg.StatusActive,
		Policy:    pkg.PolicyNotify,
		ParentApp: fmt.Sprintf("remote:%s", hostName),
	}, nil
}

// SendKeys sends input to a remote tmux session.
func (c *Client) SendKeys(hostName, tmuxTarget, text string) error {
	host, ok := c.hosts[hostName]
	if !ok {
		return fmt.Errorf("unknown host %q", hostName)
	}

	cmd := exec.Command("ssh", host.Address, "tmux", "send-keys", "-t", tmuxTarget, text, "Enter")
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("remote send-keys failed: %v", err)
	}
	return nil
}

// ReadOutput captures output from a remote tmux session.
func (c *Client) ReadOutput(hostName, tmuxTarget string, lines int) (string, error) {
	host, ok := c.hosts[hostName]
	if !ok {
		return "", fmt.Errorf("unknown host %q", hostName)
	}

	if lines <= 0 {
		lines = 50
	}
	cmd := exec.Command("ssh", host.Address, "tmux", "capture-pane", "-t", tmuxTarget, "-p", "-S", fmt.Sprintf("-%d", lines))
	out, err := cmd.Output()
	if err != nil {
		return "", fmt.Errorf("remote capture-pane failed: %v", err)
	}
	return string(out), nil
}

// DiscoverRemote finds AI CLI sessions running in tmux on a remote host.
func (c *Client) DiscoverRemote(hostName string) ([]pkg.Session, error) {
	host, ok := c.hosts[hostName]
	if !ok {
		return nil, fmt.Errorf("unknown host %q", hostName)
	}

	cmd := exec.Command("ssh", "-o", "ConnectTimeout=5", host.Address,
		"tmux", "list-panes", "-a", "-F", "#{session_name}:#{window_index}.#{pane_index} #{pane_pid} #{pane_current_command}")
	out, err := cmd.Output()
	if err != nil {
		return nil, nil
	}

	var sessions []pkg.Session
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if line == "" {
			continue
		}
		parts := strings.Fields(line)
		if len(parts) < 3 {
			continue
		}
		paneID := parts[0]
		pid := parts[1]
		cmdName := parts[2]

		for name, cliType := range pkg.KnownCLIs {
			if strings.Contains(strings.ToLower(cmdName), name) {
				sessions = append(sessions, pkg.Session{
					ID:        fmt.Sprintf("remote-%s-%s-%s", hostName, name, paneID),
					Name:      fmt.Sprintf("%s@%s [%s]", name, hostName, paneID),
					CLI:       cliType,
					Backend:   pkg.BackendTmux,
					Target:    paneID,
					Status:    pkg.StatusDiscovered,
					Policy:    pkg.PolicyNotify,
					PID:       pid,
					ParentApp: fmt.Sprintf("remote:%s", hostName),
				})
				break
			}
		}
	}

	return sessions, nil
}
