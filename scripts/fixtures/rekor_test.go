package rekortest

import (
	"crypto/ed25519"
	"crypto/x509"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestRustDeployment(t *testing.T) {
	for _, version := range []int{1, 2} {
		t.Run(fmt.Sprint(version), func(t *testing.T) {
			makeLog := NewV1
			if version == 2 {
				makeLog = NewV2
			}
			log := makeLog(t)
			root := t.TempDir()
			key, err := x509.MarshalPKCS8PrivateKey(ed25519.NewKeyFromSeed(make([]byte, ed25519.SeedSize)))
			if err != nil {
				t.Fatal(err)
			}
			if err = os.WriteFile(filepath.Join(root, "key.der"), key, 0600); err != nil {
				t.Fatal(err)
			}
			if version == 2 {
				for i := 0; i < 252; i++ {
					body := []byte(fmt.Sprintf("existing leaf %d", i))
					log.entries = append(log.entries, logged{body: body})
					log.append(body)
				}
			}
			run := func(operation string) {
				t.Helper()
				cmd := exec.Command(os.Getenv("PAYDIRT_REKOR_TEST_BIN"), "--exact", "original_rekor_server", "--ignored", "--nocapture")
				cmd.Env = append(os.Environ(), "PAYDIRT_TEST_REKOR="+fmt.Sprintf("rekor:%s?key=%s&v=%d", log.URL, log.KeyFile, version),
					"PAYDIRT_TEST_ROOT="+root, "PAYDIRT_TEST_OPERATION="+operation)
				if output, err := cmd.CombinedOutput(); err != nil {
					t.Fatalf("%s: %s: %v", operation, output, err)
				}
			}
			before := log.Len()
			run("publish")
			if log.Len()-before != 5 {
				t.Fatalf("expected 5 logged checkpoints, got %d", log.Len())
			}
			run("verify")
			requests := log.Asked()
			if version == 1 && (requests["GET /api/v1/log/proof"] == 0 || requests["POST /api/v1/index/retrieve"] == 0) {
				t.Fatal(requests)
			}
			if version == 2 && requests["GET /tile/"] == 0 {
				t.Fatal(requests)
			}
			run("tamper")
			if version == 1 {
				run("truncate")
			}
			log.Down(true)
			run("unavailable")
			log.Down(false)
			log.Fork()
			run("forked")
		})
	}
}
