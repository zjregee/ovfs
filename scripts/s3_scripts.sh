export AWS_ACCESS_KEY_ID="minioadmin"
export AWS_SECRET_ACCESS_KEY="minioadmin"
export AWS_EC2_METADATA_DISABLED="true"

aws --endpoint-url http://127.0.0.1:9000/ s3 ls s3://test
