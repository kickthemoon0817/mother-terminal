package main

import (
	"fmt"
	"os"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Println("motherctl — Mother Terminal CLI")
		fmt.Println("Usage: motherctl <command> [args]")
		fmt.Println()
		fmt.Println("Commands:")
		fmt.Println("  send <target> <query>   Send a query to a session")
		fmt.Println("  status                  Show all session statuses")
		fmt.Println("  ping <target>           Ping a session")
		os.Exit(0)
	}

	switch os.Args[1] {
	case "send":
		fmt.Println("[stub] motherctl send — not yet implemented")
	case "status":
		fmt.Println("[stub] motherctl status — not yet implemented")
	case "ping":
		fmt.Println("[stub] motherctl ping — not yet implemented")
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", os.Args[1])
		os.Exit(1)
	}
}
