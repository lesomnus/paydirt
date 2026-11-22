//go:build js && wasm

// A Go dRPC server for the binary-compatible int32 fixtures in the generated app.
package main

import (
	"context"
	drpc "github.com/lesomnus/grpc-dgram"
	"github.com/lesomnus/grpc-dgram/transport/jsport"
	"google.golang.org/grpc"
	"google.golang.org/protobuf/types/known/wrapperspb"
	"io"
	"log"
	"sync/atomic"
)

var active atomic.Int32

func collect(_ any, stream grpc.ServerStream) error {
	var sum int32
	for {
		n := new(wrapperspb.Int32Value)
		err := stream.RecvMsg(n)
		if err == io.EOF {
			return stream.SendMsg(wrapperspb.Int32(sum))
		}
		if err != nil {
			return err
		}
		sum += n.Value
	}
}
func converse(_ any, stream grpc.ServerStream) error {
	active.Add(1)
	defer active.Add(-1)
	for {
		n := new(wrapperspb.Int32Value)
		err := stream.RecvMsg(n)
		if err == io.EOF {
			return nil
		}
		if err != nil {
			return err
		}
		if err := stream.SendMsg(wrapperspb.Int32(n.Value * 2)); err != nil {
			return err
		}
	}
}
func main() {
	gateway := jsport.NewGateway()
	server := drpc.NewServer(gateway)
	server.RegisterService(&grpc.ServiceDesc{ServiceName: "app.RobotService", HandlerType: (*interface{})(nil),
		Methods: []grpc.MethodDesc{{MethodName: "InFlight", Handler: func(_ any, _ context.Context, decode func(any) error, _ grpc.UnaryServerInterceptor) (any, error) {
			if err := decode(new(wrapperspb.Int32Value)); err != nil {
				return nil, err
			}
			return wrapperspb.Int32(active.Load()), nil
		}}},
		Streams: []grpc.StreamDesc{{StreamName: "Collect", Handler: collect, ClientStreams: true}, {StreamName: "Converse", Handler: converse, ClientStreams: true, ServerStreams: true}},
	}, struct{}{})
	log.Fatal(gateway.Serve(context.Background(), server))
}
