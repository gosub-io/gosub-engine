package utils

import "encoding/json"

type StringMaybeArray struct {
	String string
	Array  []string
}

func (s *StringMaybeArray) UnmarshalJSON(data []byte) error {
	if data[0] == '[' {
		return json.Unmarshal(data, &s.Array)
	}
	return json.Unmarshal(data, &s.String)
}

func (s *StringMaybeArray) MarshalJSON() ([]byte, error) {
	if len(s.Array) > 0 {
		return json.Marshal(s.Array)
	}
	return json.Marshal(s.String)
}
