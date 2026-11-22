// Run from the pinned upstream Payday module to use its age dependency.
package main

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"filippo.io/age"
	"fmt"
	"io"
	"os"
)

func open(id age.Identity, text []byte) ([]byte, error) {
	r, err := age.Decrypt(bytes.NewReader(text), id)
	if err != nil {
		return nil, err
	}
	return io.ReadAll(r)
}
func run() error {
	var input struct {
		Master          string
		Wrapped, Sealed []byte
	}
	if err := json.NewDecoder(os.Stdin).Decode(&input); err != nil {
		return err
	}
	master, err := age.ParseX25519Identity(input.Master)
	if err != nil {
		return err
	}
	secret, err := open(master, input.Wrapped)
	if err != nil {
		return err
	}
	owner, err := age.ParseX25519Identity(string(secret))
	if err != nil {
		return err
	}
	content, err := open(owner, input.Sealed)
	if err != nil {
		return err
	}
	var doc map[string]string
	if err = json.Unmarshal(content, &doc); err != nil {
		return err
	}
	if doc["value"] != "ZnJvbSBSdXN0" {
		return fmt.Errorf("incorrect Rust payload")
	}
	var buffer bytes.Buffer
	w, err := age.Encrypt(&buffer, owner.Recipient())
	if err != nil {
		return err
	}
	if _, err = w.Write([]byte(`{"value":"ZnJvbSBHbw=="}`)); err != nil {
		return err
	}
	if err = w.Close(); err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(base64.StdEncoding.EncodeToString(buffer.Bytes()))
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
