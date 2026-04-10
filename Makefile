PORT ?= 8066

dev:
	python3 -c 'import run; run.dev()'

.PHONY: server
server:
	@echo "serving on http://localhost:$(PORT)/viz/"
	python3 -m http.server $(PORT)
