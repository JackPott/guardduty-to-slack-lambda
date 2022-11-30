#!/bin/bash
file=$(shuf -ezn 1 examples/* | xargs -0 -n1)
echo $file
cat $file | docker run -i --rm \
    -v ${PWD}/target/x86_64-unknown-linux-musl/release:/var/task \
    -e AWS_DEFAULT_REGION=${AWS_REGION} \
    -e AWS_ACCESS_KEY_ID=${AWS_ACCESS_KEY_ID} \
    -e AWS_SECRET_ACCESS_KEY=${AWS_SECRET_ACCESS_KEY} \
    -e region="eu-west-2" \
    -e DOCKER_LAMBDA_USE_STDIN=1 \
    --env-file .env \
    lambci/lambda:provided bootstrap
