// Package nebulacore: ядро Nebula для Android-приложения Nebula Client.
// Nebula работает внутри процесса приложения, адаптер (tun) ей отдаёт VpnService в виде файлового дескриптора.
package nebulacore

import (
	"crypto/rand"
	"encoding/json"
	"errors"
	"strings"
	"sync"

	"github.com/sirupsen/logrus"
	"github.com/slackhq/nebula"
	"github.com/slackhq/nebula/cert"
	"github.com/slackhq/nebula/config"
	"github.com/slackhq/nebula/overlay"
	"golang.org/x/crypto/curve25519"
)

// Версия nebula должна совпадать с десктопной (см. NEBULA_VERSION в crates/service/src/nebula.rs).
const NebulaVersion = "1.9.3"

var (
	mu      sync.Mutex
	control *nebula.Control
	logbuf  = &ring{max: 64 * 1024}
	logger  = newLogger()
)

// ring хранит хвост журнала, чтобы показать его в приложении.
type ring struct {
	mu  sync.Mutex
	max int
	buf []byte
}

func (r *ring) Write(p []byte) (int, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.buf = append(r.buf, p...)
	if len(r.buf) > r.max {
		r.buf = append([]byte(nil), r.buf[len(r.buf)-r.max:]...)
	}
	return len(p), nil
}

func (r *ring) String() string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return string(r.buf)
}

func newLogger() *logrus.Logger {
	l := logrus.New()
	l.SetOutput(logbuf)
	return l
}

// Keypair: ключ устройства в PEM, как у nebula-cert keygen. Приватный остаётся на телефоне, панели уходит публичный.
type Keypair struct {
	Private string
	Public  string
}

func GenerateKeypair() (*Keypair, error) {
	priv := make([]byte, 32)
	if _, err := rand.Read(priv); err != nil {
		return nil, err
	}
	pub, err := curve25519.X25519(priv, curve25519.Basepoint)
	if err != nil {
		return nil, err
	}
	return &Keypair{
		Private: string(cert.MarshalX25519PrivateKey(priv)),
		Public:  string(cert.MarshalX25519PublicKey(pub)),
	}, nil
}

// Start запускает nebula с конфигом (YAML, ключи и сертификаты внутри) и дескриптором tun от VpnService.
func Start(configYaml string, tunFd int) error {
	mu.Lock()
	defer mu.Unlock()
	if control != nil {
		return errors.New("nebula already running")
	}
	c := config.NewC(logger)
	if err := c.LoadString(configYaml); err != nil {
		return err
	}
	fd := tunFd
	ctrl, err := nebula.Main(c, false, "Android "+NebulaVersion, logger, overlay.NewFdDeviceFromConfig(&fd))
	if err != nil {
		return err
	}
	ctrl.Start()
	control = ctrl
	return nil
}

func Stop() {
	mu.Lock()
	c := control
	control = nil
	mu.Unlock()
	if c != nil {
		c.Stop()
	}
}

func Running() bool {
	mu.Lock()
	defer mu.Unlock()
	return control != nil
}

// RebindUDP зовут при смене сети (Wi-Fi <-> мобильная), чтобы сокет переехал на новый интерфейс.
func RebindUDP() {
	mu.Lock()
	c := control
	mu.Unlock()
	if c != nil {
		c.RebindUDPServer()
	}
}

// Tunnels: число установленных туннелей до других узлов.
func Tunnels() int {
	mu.Lock()
	c := control
	mu.Unlock()
	if c == nil {
		return 0
	}
	return len(c.ListHostmapHosts(false))
}

// TunnelIPs: JSON-массив адресов, с которыми сейчас есть туннель.
func TunnelIPs() string {
	mu.Lock()
	c := control
	mu.Unlock()
	out := []string{}
	if c != nil {
		for _, h := range c.ListHostmapHosts(false) {
			out = append(out, h.VpnIp.String())
		}
	}
	b, _ := json.Marshal(out)
	return string(b)
}

func Logs() string { return logbuf.String() }

func Version() string { return strings.TrimSpace(NebulaVersion) }
