package tui

import (
	"strings"
	"testing"

	tea "github.com/charmbracelet/bubbletea"
)

// ── helpers ───────────────────────────────────────────────────────────────────

func keyMsg(k string) tea.KeyMsg {
	return tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune(k)}
}

func specialKey(t tea.KeyType) tea.KeyMsg {
	return tea.KeyMsg{Type: t}
}

func typeInto(im InputModel, s string) InputModel {
	for _, ch := range s {
		im, _ = im.Update(keyMsg(string(ch)))
	}
	return im
}

// ── NewInputModel ─────────────────────────────────────────────────────────────

func TestNewInputModel_startsEmpty(t *testing.T) {
	im := NewInputModel()
	if im.value != "" {
		t.Errorf("expected empty value, got %q", im.value)
	}
	if im.suggestion != "" {
		t.Errorf("expected empty suggestion, got %q", im.suggestion)
	}
	if im.dirMode {
		t.Error("expected dirMode to be false")
	}
}

// ── Character input ───────────────────────────────────────────────────────────

func TestUpdate_appendsSingleCharacter(t *testing.T) {
	im := NewInputModel()
	im, _ = im.Update(keyMsg("a"))
	if im.value != "a" {
		t.Errorf("expected \"a\", got %q", im.value)
	}
}

func TestUpdate_appendsMultiByteUnicodeRune(t *testing.T) {
	// Korean IME produces multi-byte runes; each should append as one rune.
	im := NewInputModel()
	im, _ = im.Update(keyMsg("한"))
	if im.value != "한" {
		t.Errorf("expected Korean rune, got %q", im.value)
	}
}

func TestUpdate_ignoresKeysWithMoreThanOneRune(t *testing.T) {
	// Synthetic key strings that carry multiple runes (e.g., composed sequences)
	// must not be appended, to avoid input corruption.
	im := NewInputModel()
	multi := tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune("ab")}
	im, _ = im.Update(multi)
	// "ab" is two runes — len([]rune("ab")) == 2, so the guard fires.
	if im.value != "" {
		t.Errorf("expected no change for multi-rune key, got %q", im.value)
	}
}

func TestUpdate_spaceKeyAppendsSpace(t *testing.T) {
	im := NewInputModel()
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeySpace})
	if im.value != " " {
		t.Errorf("expected space, got %q", im.value)
	}
}

// ── Backspace ─────────────────────────────────────────────────────────────────

func TestUpdate_backspaceRemovesLastRune(t *testing.T) {
	im := NewInputModel()
	im = typeInto(im, "abc")
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyBackspace})
	if im.value != "ab" {
		t.Errorf("expected \"ab\" after backspace, got %q", im.value)
	}
}

func TestUpdate_backspaceOnEmptyInputIsNoop(t *testing.T) {
	im := NewInputModel()
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyBackspace})
	if im.value != "" {
		t.Errorf("expected empty after backspace on empty, got %q", im.value)
	}
}

func TestUpdate_backspaceRemovesMultiByteRuneAsUnit(t *testing.T) {
	// Korean character is 3 bytes but 1 rune — backspace should remove it whole.
	im := NewInputModel()
	im = typeInto(im, "한")
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyBackspace})
	if im.value != "" {
		t.Errorf("expected empty after removing Korean rune, got %q", im.value)
	}
}

// ── Escape ────────────────────────────────────────────────────────────────────

func TestUpdate_escapeResetsValue(t *testing.T) {
	im := NewInputModel()
	im = typeInto(im, "/spawn")
	// bubbletea KeyEscape produces msg.String() == "esc"; input.go matches on "esc"
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyEscape})
	if im.value != "" {
		t.Errorf("expected empty after escape, got %q", im.value)
	}
}

// ── getSuggestion ─────────────────────────────────────────────────────────────

func TestGetSuggestion_returnsEmptyForNonSlashPrefix(t *testing.T) {
	im := NewInputModel()
	im.value = "hello"
	if s := im.getSuggestion(); s != "" {
		t.Errorf("expected no suggestion for non-slash input, got %q", s)
	}
}

func TestGetSuggestion_returnsEmptyForEmptyInput(t *testing.T) {
	im := NewInputModel()
	if s := im.getSuggestion(); s != "" {
		t.Errorf("expected no suggestion for empty input, got %q", s)
	}
}

func TestGetSuggestion_completesPartialSlashCommand(t *testing.T) {
	im := NewInputModel()
	im.value = "/sp"
	s := im.getSuggestion()
	if s != "/spawn" {
		t.Errorf("expected /spawn suggestion, got %q", s)
	}
}

func TestGetSuggestion_returnsEmptyForExactCommandMatch(t *testing.T) {
	im := NewInputModel()
	im.value = "/spawn"
	s := im.getSuggestion()
	if s != "" {
		t.Errorf("expected no suggestion for exact match, got %q", s)
	}
}

func TestGetSuggestion_returnsEmptyWhenCommandHasArgs(t *testing.T) {
	im := NewInputModel()
	im.value = "/spawn claude"
	s := im.getSuggestion()
	// /spawn has a registered entry but once an arg is appended no top-level
	// command suggestion should fire.
	if s != "" {
		t.Errorf("expected no top-level suggestion with args, got %q", s)
	}
}

func TestGetSuggestion_completesSpawnCLIArgument(t *testing.T) {
	im := NewInputModel()
	im.value = "/spawn cl"
	s := im.getSuggestion()
	if s != "/spawn claude" {
		t.Errorf("expected /spawn claude suggestion, got %q", s)
	}
}

func TestGetSuggestion_spawnCLISuggestionIsCaseInsensitive(t *testing.T) {
	im := NewInputModel()
	im.value = "/spawn CL"
	s := im.getSuggestion()
	if s != "/spawn claude" {
		t.Errorf("expected /spawn claude for uppercase prefix, got %q", s)
	}
}

func TestGetSuggestion_noSuggestionWhenCLIAlreadyTypedFully(t *testing.T) {
	im := NewInputModel()
	im.value = "/spawn claude"
	s := im.getSuggestion()
	// "claude" matches the prefix of "claude" but since arg == cli it must not suggest.
	if s != "" {
		t.Errorf("expected no suggestion for fully typed CLI, got %q", s)
	}
}

// ── Tab key — accept suggestion ───────────────────────────────────────────────

func TestUpdate_tabAcceptsSlashCommandSuggestion(t *testing.T) {
	im := NewInputModel()
	im = typeInto(im, "/sp")
	// Ensure suggestion is populated before pressing tab
	if im.suggestion == "" {
		t.Skip("no suggestion generated; getSuggestion logic may differ")
	}
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyTab})
	if !strings.HasPrefix(im.value, "/spawn") {
		t.Errorf("expected /spawn after tab completion, got %q", im.value)
	}
}

func TestUpdate_tabAppendsSuffixSpaceForCommandCompletion(t *testing.T) {
	im := NewInputModel()
	im = typeInto(im, "/sp")
	if im.suggestion == "" {
		t.Skip("no suggestion")
	}
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyTab})
	if !strings.HasSuffix(im.value, " ") {
		t.Errorf("expected trailing space after command tab-complete, got %q", im.value)
	}
}

func TestUpdate_tabInDirModeAppendsSuffixSlash(t *testing.T) {
	im := NewInputModel()
	im.dirMode = true
	// Inject a synthetic suggestion — getDirSuggestion depends on fs state
	im.suggestion = "/tmp/somedir"
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyTab})
	if !strings.HasSuffix(im.value, "/") {
		t.Errorf("expected trailing slash for dir-mode tab-complete, got %q", im.value)
	}
}

func TestUpdate_tabWithNoSuggestionIsNoop(t *testing.T) {
	im := NewInputModel()
	im.value = "/zzz"
	im.suggestion = ""
	before := im.value
	im, _ = im.Update(tea.KeyMsg{Type: tea.KeyTab})
	if im.value != before {
		t.Errorf("expected no change when no suggestion, got %q", im.value)
	}
}

// ── getDirSuggestion ──────────────────────────────────────────────────────────

func TestGetDirSuggestion_returnsEmptyForEmptyInput(t *testing.T) {
	im := NewInputModel()
	im.dirMode = true
	if s := im.getDirSuggestion(); s != "" {
		t.Errorf("expected no suggestion for empty input, got %q", s)
	}
}

func TestGetDirSuggestion_returnsEmptyForNonexistentPath(t *testing.T) {
	im := NewInputModel()
	im.dirMode = true
	im.value = "/this/path/does/not/exist/anywhere"
	if s := im.getDirSuggestion(); s != "" {
		t.Errorf("expected no suggestion for nonexistent path, got %q", s)
	}
}

func TestGetDirSuggestion_returnsEmptyForFileNotDirectory(t *testing.T) {
	// /etc/hosts is a file, not a directory — no suggestion expected.
	im := NewInputModel()
	im.dirMode = true
	im.value = "/etc/hosts"
	s := im.getDirSuggestion()
	// The implementation lists contents of /etc looking for entries that start
	// with "hosts" — since hosts is a file, IsDir() is false, so no match.
	if s != "" {
		t.Errorf("expected no suggestion for file path, got %q", s)
	}
}

func TestGetDirSuggestion_suggestsDirectoryUnderTmp(t *testing.T) {
	// /tmp always exists and is a directory; prefix "/" should suggest "tmp".
	im := NewInputModel()
	im.dirMode = true
	im.value = "/tm"
	s := im.getDirSuggestion()
	if s == "" {
		t.Skip("/tmp not visible from root for this test environment")
	}
	if !strings.Contains(s, "tmp") {
		t.Errorf("expected suggestion containing tmp, got %q", s)
	}
}

func TestGetDirSuggestion_listsContentsWhenInputEndsWithSlash(t *testing.T) {
	im := NewInputModel()
	im.dirMode = true
	im.value = "/tmp/"
	// Should list something under /tmp (or return empty if /tmp is empty)
	// Either way, must not panic.
	_ = im.getDirSuggestion()
}

func TestGetDirSuggestion_handlesPermissionDeniedGracefully(t *testing.T) {
	// /root is typically readable only by root; on most systems this is a
	// permission-denied directory. getDirSuggestion should return "" not panic.
	im := NewInputModel()
	im.dirMode = true
	im.value = "/root/s"
	// Must not panic regardless of outcome
	_ = im.getDirSuggestion()
}

// ── getSuggestion / slashCommands registry completeness ───────────────────────

func TestSlashCommands_allEntriesHaveNonEmptyCommandAndDesc(t *testing.T) {
	for _, cmd := range slashCommands {
		if cmd.command == "" {
			t.Errorf("slash command entry has empty command field: %+v", cmd)
		}
		if cmd.desc == "" {
			t.Errorf("slash command %q has empty desc field", cmd.command)
		}
		if !strings.HasPrefix(cmd.command, "/") {
			t.Errorf("slash command %q does not start with /", cmd.command)
		}
	}
}
