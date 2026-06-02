package handlers

import (
	"errors"
	"net/http"
	"strconv"

	"github.com/gin-gonic/gin"
	"github.com/yourusername/gpay-remit/logger"
	"github.com/yourusername/gpay-remit/models"
	"golang.org/x/crypto/bcrypt"
	"gorm.io/gorm"
)

type UserHandler struct {
	db *gorm.DB
}

func NewUserHandler(db *gorm.DB) *UserHandler {
	return &UserHandler{db: db}
}

type UpdateUserRequest struct {
	Name            *string `json:"name,omitempty" binding:"omitempty,min=2,max=255"`
	Country         *string `json:"country,omitempty" binding:"omitempty,len=2"`
	DefaultCurrency *string `json:"default_currency,omitempty" binding:"omitempty,min=2,max=10"`
	Password        *string `json:"password,omitempty" binding:"omitempty,min=8"`
}

func (h *UserHandler) authorizeSelfOrAdmin(c *gin.Context, targetUserID uint) bool {
	claimsRaw, exists := c.Get("claims")
	if !exists {
		return false
	}
	
	// Check if claims is a map or a struct depending on your Auth logic
	// In the standard JwtAuthMiddleware, claims might be a custom struct or map
	// Since I don't see the exact claims struct, I'll extract user_id from gin context if set directly
	userIDRaw, exists := c.Get("user_id")
	if !exists {
		return false
	}
	
	var requesterID uint
	switch v := userIDRaw.(type) {
	case uint:
		requesterID = v
	case float64:
		requesterID = uint(v)
	case int:
		requesterID = uint(v)
	default:
		return false
	}

	roleRaw, roleExists := c.Get("role")
	role := ""
	if roleExists {
		if r, ok := roleRaw.(string); ok {
			role = r
		}
	}

	if role == "admin" || requesterID == targetUserID {
		return true
	}
	
	return false
}

func (h *UserHandler) GetUser(c *gin.Context) {
	idStr := c.Param("id")
	id, err := strconv.ParseUint(idStr, 10, 32)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid user ID"})
		return
	}

	if !h.authorizeSelfOrAdmin(c, uint(id)) {
		c.JSON(http.StatusForbidden, gin.H{"error": "Forbidden: Can only view your own profile"})
		return
	}

	var user models.User
	if err := h.db.First(&user, id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			c.JSON(http.StatusNotFound, gin.H{"error": "User not found"})
		} else {
			logger.Log.WithField("error", err).Error("Failed to fetch user")
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to fetch user"})
		}
		return
	}

	c.JSON(http.StatusOK, gin.H{"user": user})
}

func (h *UserHandler) UpdateUser(c *gin.Context) {
	idStr := c.Param("id")
	id, err := strconv.ParseUint(idStr, 10, 32)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid user ID"})
		return
	}

	if !h.authorizeSelfOrAdmin(c, uint(id)) {
		c.JSON(http.StatusForbidden, gin.H{"error": "Forbidden: Can only update your own profile"})
		return
	}

	var req UpdateUserRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid request payload", "details": err.Error()})
		return
	}

	var user models.User
	if err := h.db.First(&user, id).Error; err != nil {
		if errors.Is(err, gorm.ErrRecordNotFound) {
			c.JSON(http.StatusNotFound, gin.H{"error": "User not found"})
		} else {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to fetch user"})
		}
		return
	}

	// Apply updates
	if req.Name != nil {
		user.Name = *req.Name
	}
	if req.Country != nil {
		user.Country = *req.Country
	}
	if req.DefaultCurrency != nil {
		user.DefaultCurrency = *req.DefaultCurrency
	}
	if req.Password != nil && *req.Password != "" {
		if err := models.ValidatePasswordStrength(*req.Password); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "Password is too weak", "details": err.Error()})
			return
		}
		hash, err := bcrypt.GenerateFromPassword([]byte(*req.Password), 12)
		if err != nil {
			c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to hash password"})
			return
		}
		user.PasswordHash = string(hash)
	}

	if err := h.db.Save(&user).Error; err != nil {
		logger.Log.WithField("error", err).Error("Failed to update user")
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to update user"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "User updated successfully", "user": user})
}

func (h *UserHandler) DeleteUser(c *gin.Context) {
	idStr := c.Param("id")
	id, err := strconv.ParseUint(idStr, 10, 32)
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "Invalid user ID"})
		return
	}

	if !h.authorizeSelfOrAdmin(c, uint(id)) {
		c.JSON(http.StatusForbidden, gin.H{"error": "Forbidden: Can only delete your own profile"})
		return
	}

	// Use GORM's Soft Delete
	if err := h.db.Delete(&models.User{}, id).Error; err != nil {
		logger.Log.WithField("error", err).Error("Failed to delete user")
		c.JSON(http.StatusInternalServerError, gin.H{"error": "Failed to delete user"})
		return
	}

	c.JSON(http.StatusOK, gin.H{"message": "User deleted successfully"})
}
