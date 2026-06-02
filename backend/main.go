package main

import (
	"context"
	"errors"
	"net/http"
	"os"
	"os/signal"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/yourusername/gpay-remit/config"
	"github.com/yourusername/gpay-remit/handlers"
	"github.com/yourusername/gpay-remit/logger"
	"github.com/yourusername/gpay-remit/middleware"
	"github.com/yourusername/gpay-remit/workers"
)

func main() {
	env := os.Getenv("APP_ENV")
	logger.Init(env)

	cfg, err := config.LoadConfig()
	if err != nil {
		logger.Log.WithField("error", err).Fatal("Failed to load config")
	}

	db, err := config.InitDB(cfg)
	if err != nil {
		logger.Log.WithField("error", err).Fatal("Failed to connect to database")
	}

	router := gin.New()
	router.Use(gin.Recovery())
	router.Use(middleware.RequestIDMiddleware())
	router.Use(middleware.RequestLogger())
	router.Use(middleware.ErrorHandler())

	// CORS middleware
	router.Use(func(c *gin.Context) {
		c.Writer.Header().Set("Access-Control-Allow-Origin", "*")
		c.Writer.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
		c.Writer.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization")
		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(204)
			return
		}
		c.Next()
	})

	healthHandler := handlers.NewHealthHandler(db, cfg)
	router.GET("/health", healthHandler.Health)
	router.GET("/health/ready", healthHandler.Ready)
	router.GET("/health/live", healthHandler.Live)

	api := router.Group("/api/v1")
	{
		authHandler := handlers.NewAuthHandler(db, cfg)
		api.POST("/auth/register", authHandler.Register)
		api.POST("/auth/login", authHandler.Login)
		api.POST("/auth/refresh", authHandler.Refresh)

		api.POST("/users", authHandler.Register)

		protected := api.Group("/")
		protected.Use(middleware.JwtAuthMiddleware(cfg))
		protected.Use(middleware.AuditTrail(db))
		{
			remittanceHandler := handlers.NewRemittanceHandler(db, cfg)
			protected.POST("/remittances/create", remittanceHandler.CreateRemittance)
			protected.POST("/remittances", remittanceHandler.SendRemittance)
			protected.GET("/remittances/:id", remittanceHandler.GetRemittance)
			protected.GET("/remittances", remittanceHandler.ListRemittances)
			protected.POST("/remittances/:id/complete", middleware.RequireRole("admin"), remittanceHandler.CompleteRemittance)

			protected.POST("/invoices", remittanceHandler.CreateInvoice)
			protected.GET("/invoices/:id", remittanceHandler.GetInvoice)

			feeService := services.NewFeeService(cfg)
			feeHandler := handlers.NewFeeHandler(feeService)
			protected.GET("/fees/calculate", feeHandler.Calculate)

			auditHandler := handlers.NewAuditLogHandler(db)
			protected.GET("/audit/logs", middleware.RequireRole("admin"), auditHandler.List)

			userHandler := handlers.NewUserHandler(db)
			protected.GET("/users/:id", userHandler.GetUser)
			protected.PUT("/users/:id", userHandler.UpdateUser)
			protected.DELETE("/users/:id", userHandler.DeleteUser)
		}
	}

	port := cfg.Port
	if port == "" {
		port = "8080"
	}

	var shuttingDown atomic.Bool
	router.Use(func(c *gin.Context) {
		if shuttingDown.Load() {
			c.AbortWithStatusJSON(http.StatusServiceUnavailable, gin.H{
				"error": "server is shutting down",
			})
			return
		}
		c.Next()
	})

	server := &http.Server{
		Addr:    ":" + port,
		Handler: router,
	}

	baseCtx, cancelWorkers := context.WithCancel(context.Background())
	var wg sync.WaitGroup
	workers.StartMonitor(baseCtx, &wg)

	errCh := make(chan error, 1)
	go func() {
		logger.Log.WithField("port", port).Info("Starting Gpay-Remit API server")
		if err := server.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
			errCh <- err
		}
	}()

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM)

	select {
	case sig := <-sigCh:
		shuttingDown.Store(true)
		logger.Log.WithField("signal", sig.String()).Info("Shutdown signal received")
	case err := <-errCh:
		logger.Log.WithField("error", err).Fatal("Server failed unexpectedly")
	}

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	logger.Log.Info("Stopping background workers...")
	cancelWorkers()

	logger.Log.Info("Shutting down HTTP server...")
	if err := server.Shutdown(shutdownCtx); err != nil {
		logger.Log.WithField("error", err).Error("HTTP server shutdown error")
	} else {
		logger.Log.Info("HTTP server stopped accepting new requests")
	}

	done := make(chan struct{})
	go func() {
		wg.Wait()
		close(done)
	}()

	select {
	case <-done:
		logger.Log.Info("Background workers stopped")
	case <-shutdownCtx.Done():
		logger.Log.Warn("Timeout waiting for background workers to stop")
	}

	logger.Log.Info("Closing database connection...")
	if sqlDB, err := db.DB(); err != nil {
		logger.Log.WithField("error", err).Error("Failed to get sql.DB for closing")
	} else if err := sqlDB.Close(); err != nil {
		logger.Log.WithField("error", err).Error("Failed to close database connection")
	} else {
		logger.Log.Info("Database connection closed")
	}
}
