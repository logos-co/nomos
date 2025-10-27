{{- define "nomos-runner.chart" -}}
{{- .Chart.Name -}}
{{- end -}}

{{- define "nomos-runner.fullname" -}}
{{- printf "%s" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "nomos-runner.labels" -}}
app.kubernetes.io/name: {{ include "nomos-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "nomos-runner.selectorLabels" -}}
app.kubernetes.io/name: {{ include "nomos-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "nomos-runner.validatorLabels" -}}
{{- $root := index . "root" -}}
{{- $index := index . "index" -}}
app.kubernetes.io/name: {{ include "nomos-runner.chart" $root }}
app.kubernetes.io/instance: {{ $root.Release.Name }}
nomos/logical-role: validator
nomos/validator-index: "{{ $index }}"
{{- end -}}

{{- define "nomos-runner.executorLabels" -}}
{{- $root := index . "root" -}}
{{- $index := index . "index" -}}
app.kubernetes.io/name: {{ include "nomos-runner.chart" $root }}
app.kubernetes.io/instance: {{ $root.Release.Name }}
nomos/logical-role: executor
nomos/executor-index: "{{ $index }}"
{{- end -}}

{{- define "nomos-runner.prometheusLabels" -}}
app.kubernetes.io/name: {{ include "nomos-runner.chart" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
nomos/logical-role: prometheus
{{- end -}}
