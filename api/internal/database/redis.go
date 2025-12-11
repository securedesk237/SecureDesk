package database

import (
	"context"

	"github.com/redis/go-redis/v9"
)

func ConnectRedis(url string, password string) *redis.Client {
	client := redis.NewClient(&redis.Options{
		Addr:     url,
		Password: password,
		DB:       0,
	})

	// Test connection
	ctx := context.Background()
	if err := client.Ping(ctx).Err(); err != nil {
		// Log but don't fail - Redis is optional for caching
		println("Warning: Redis connection failed:", err.Error())
	}

	return client
}
