// magpie_golden 用固定测试密钥驱动 Magpie 的脱敏代码，对语料生成占位符
// golden JSON（开发期工具，不进产物）。用法：
//
//	go run ./tools/magpie_golden -corpus <corpus.json> -out <golden.json>
//
// 语料格式：["plain text containing one detectable value", ...]。
// 每条经 redact.Mask 脱敏，从输出中提取占位符，kind 取自占位符本身，
// 原值经 redact.Value 反查核对。输出 [{kind, value, placeholder}]，
// Rust 侧用同一固定密钥逐条比对 tag 输出。
package main

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/yetone/magpie/internal/redact"
)

type goldenEntry struct {
	Kind        string `json:"kind"`
	Value       string `json:"value"`
	Placeholder string `json:"placeholder"`
}

// fixedKeyHex 与 Rust 侧 golden 测试共享（64 个十六进制字符 = 32 字节）。
const fixedKeyHex = "7f3a9c1d5e2b8406ad17fe93c2b60845d1a4f7e29b3c5d8a06f4e7b2c9d1a305"

var placeholderRe = regexp.MustCompile(`\{\{[A-Z][A-Z0-9_]*_[a-z2-7]{8}\}\}`)

func main() {
	corpusPath := flag.String("corpus", "", "corpus json path (array of strings)")
	outPath := flag.String("out", "", "output golden json path")
	flag.Parse()
	if *corpusPath == "" || *outPath == "" {
		flag.Usage()
		os.Exit(2)
	}

	key, err := hexDecode(fixedKeyHex)
	if err != nil || len(key) != 32 {
		panic("bad fixed key")
	}
	tmp, err := os.MkdirTemp("", "magpie-golden-key")
	if err != nil {
		panic(err)
	}
	defer os.RemoveAll(tmp)
	keyPath := filepath.Join(tmp, "key")
	if err := os.WriteFile(keyPath, key, 0o600); err != nil {
		panic(err)
	}
	redact.SetKeyPath(keyPath)

	raw, err := os.ReadFile(*corpusPath)
	if err != nil {
		panic(err)
	}
	var corpus []string
	if err := json.Unmarshal(raw, &corpus); err != nil {
		panic(err)
	}

	out := make([]goldenEntry, 0, len(corpus))
	for _, text := range corpus {
		masked, _ := redact.Mask(text, redact.Options{Secrets: true, Personal: true})
		for _, match := range placeholderRe.FindAllString(masked, -1) {
			trimmed := strings.TrimSuffix(strings.TrimPrefix(match, "{{"), "}}")
			underscore := strings.LastIndex(trimmed, "_")
			if underscore < 1 {
				panic("malformed placeholder " + match)
			}
			kind := trimmed[:underscore]
			value, ok := redact.Value(match)
			if !ok {
				panic("value table miss for " + match)
			}
			out = append(out, goldenEntry{Kind: kind, Value: value, Placeholder: match})
		}
	}

	blob, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		panic(err)
	}
	if err := os.WriteFile(*outPath, blob, 0o644); err != nil {
		panic(err)
	}
	fmt.Printf("wrote %d golden entries to %s\n", len(out), *outPath)
}

func hexDecode(s string) ([]byte, error) {
	if len(s)%2 != 0 {
		return nil, fmt.Errorf("odd hex")
	}
	out := make([]byte, len(s)/2)
	for i := 0; i < len(out); i++ {
		hi := hexVal(s[i*2])
		lo := hexVal(s[i*2+1])
		if hi < 0 || lo < 0 {
			return nil, fmt.Errorf("bad hex")
		}
		out[i] = byte(hi<<4 | lo)
	}
	return out, nil
}

func hexVal(c byte) int {
	switch {
	case c >= '0' && c <= '9':
		return int(c - '0')
	case c >= 'a' && c <= 'f':
		return int(c-'a') + 10
	case c >= 'A' && c <= 'F':
		return int(c-'A') + 10
	}
	return -1
}
