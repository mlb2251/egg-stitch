PORT ?= 8000

.PHONY: server
server:
	@echo "serving on http://localhost:$(PORT)/viz/"
	python3 -m http.server $(PORT)
