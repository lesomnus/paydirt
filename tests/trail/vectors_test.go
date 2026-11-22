// Mounted with go test -overlay into the pinned upstream trail package.
package trail

import (
	"bytes"
	"compress/gzip"
	"crypto/ed25519"
	"encoding/json"
	"os"
	"testing"
	"time"

	"github.com/lesomnus/flob"
	"github.com/lesomnus/payday/audit"
	"github.com/lesomnus/payday/pdid"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/wrapperspb"
)

func TestPaydirtVectors(t *testing.T) {
	must := func(err error) {
		t.Helper()
		if err != nil {
			t.Fatal(err)
		}
	}
	object, err := pdid.From([]byte{1, 2, 3, 4, 5, 6, 128, 8, 128, 7, 11, 12, 13, 14, 15, 16})
	must(err)
	key := bytes.Repeat([]byte{37}, 32)
	d, err := audit.NewDigest(key)
	must(err)
	line := audit.Line{Id: [16]byte{1, 2, 3, 4, 5, 6, 128, 8, 128, 3, 11, 12, 13, 14, 15, 16}, Created: time.UnixMicro(1700000000123456).UTC(), Action: "/app.RobotService/Patch"}
	row := wrapperspb.String("sample")
	wire, err := proto.Marshal(row)
	must(err)
	digest, seal, err := d.OfSealed(object, row, []uint32{1}, []uint32{1}, line)
	must(err)
	archived := Archived{Key: "row-1", Namespace: "tenant", Digest: string(flob.DigestFromBytes(wire)), Intent: "archive", State: Present, Labels: map[string]string{"owner": "tenant", "z": "한글"}}
	signer := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{23}, 32))
	statementBytes := statement(archived)
	pk, sig, err := sign(signer, statementBytes)
	must(err)
	makePoint := func(k int, prev flob.Digest) []byte {
		c := checkpointOf(k, line.Created, []Archived{archived})
		c.Prev = prev
		b, err := json.Marshal(c)
		must(err)
		pub, s, err := sign(signer, b)
		must(err)
		doc, err := json.Marshal(signed{Checkpoint: b, Key: pub, Sig: s})
		must(err)
		var out bytes.Buffer
		z := gzip.NewWriter(&out)
		_, err = z.Write(doc)
		must(err)
		must(z.Close())
		return out.Bytes()
	}
	first := makePoint(1, "")
	second := makePoint(2, flob.DigestFromBytes(first))
	v := map[string]any{"key": key, "object": object.Bytes(), "line": line.Id[:], "action": line.Action, "micros": line.Created.UnixMicro(), "wire": wire, "digest": digest, "seal": seal, "gone": d.Gone(object, line), "forgotten": d.Forgotten(object), "statement": statementBytes, "pk": pk, "sig": sig, "checkpoints": [][]byte{first, second}}
	b, err := json.MarshalIndent(v, "", "  ")
	must(err)
	must(os.WriteFile(os.Getenv("PAYDIRT_TRAIL_VECTORS"), append(b, '\n'), 0644))
}
