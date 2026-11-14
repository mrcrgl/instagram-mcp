{{- define "instagram-mcp.selectorLabels" -}}
app.kubernetes.io/name: instagram-mcp
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "instagram-mcp.labels" -}}
{{ include "instagram-mcp.selectorLabels" . }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
{{- end }}

{{/* The rmcp Host allowlist comes from this URL, so it must not be empty. */}}
{{- define "instagram-mcp.host" -}}
{{- $url := required "publicBaseUrl is required" .Values.publicBaseUrl -}}
{{- (urlParse $url).host -}}
{{- end }}
