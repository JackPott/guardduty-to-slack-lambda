all: build run

build: 
	scripts/build.sh
run:
	scripts/run_local.sh
release: build
	scripts/release.sh
test:
	echo "TODO: Write unit tests!"
