/**
 * Ferrous Network - Shared JavaScript Utilities
 * Common functions for all Ferrous web interfaces
 */

(function(global) {
  'use strict';

  // ═══════════════════════════════════════════════════════════
  // FERROUS NAMESPACE
  // ═══════════════════════════════════════════════════════════
  const Ferrous = {};

  // ═══════════════════════════════════════════════════════════
  // FORMATTING UTILITIES
  // ═══════════════════════════════════════════════════════════

  /**
   * Format a number with locale-aware separators
   * @param {number} num - Number to format
   * @param {number} decimals - Decimal places (optional)
   * @returns {string} Formatted number
   */
  Ferrous.formatNumber = function(num, decimals) {
    if (typeof decimals === 'number') {
      return num.toLocaleString(undefined, {
        minimumFractionDigits: decimals,
        maximumFractionDigits: decimals
      });
    }
    return num.toLocaleString();
  };

  /**
   * Generate a random hex string
   * @param {number} length - Length of hex string
   * @returns {string} Random hex string
   */
  Ferrous.randomHex = function(length) {
    const chars = '0123456789abcdef';
    let result = '';
    for (let i = 0; i < length; i++) {
      result += chars[Math.floor(Math.random() * 16)];
    }
    return result;
  };

  /**
   * Shorten a hash for display
   * @param {string} hash - Full hash
   * @param {number} startChars - Characters to show at start (default 8)
   * @param {number} endChars - Characters to show at end (default 6)
   * @returns {string} Shortened hash
   */
  Ferrous.shortHash = function(hash, startChars, endChars) {
    startChars = startChars || 8;
    endChars = endChars || 6;
    if (!hash || hash.length <= startChars + endChars) return hash;
    return hash.slice(0, startChars) + '...' + hash.slice(-endChars);
  };

  /**
   * Shorten an address for display
   * @param {string} address - Full address
   * @returns {string} Shortened address
   */
  Ferrous.shortAddr = function(address) {
    if (!address || address.length <= 10) return address;
    return address.slice(0, 6) + '...' + address.slice(-4);
  };

  /**
   * Format a timestamp as relative time
   * @param {number} seconds - Seconds ago
   * @returns {string} Human-readable time ago
   */
  Ferrous.timeAgo = function(seconds) {
    if (seconds < 0) seconds = 0;
    if (seconds < 60) return seconds + 's ago';
    if (seconds < 3600) return Math.floor(seconds / 60) + 'm ago';
    if (seconds < 86400) {
      const hours = Math.floor(seconds / 3600);
      const mins = Math.floor((seconds % 3600) / 60);
      return hours + 'h ' + mins + 'm ago';
    }
    return Math.floor(seconds / 86400) + 'd ago';
  };

  /**
   * Format a Unix timestamp to UTC string
   * @param {number} timestamp - Unix timestamp (seconds)
   * @returns {string} Formatted UTC datetime
   */
  Ferrous.formatTimestamp = function(timestamp) {
    return new Date(timestamp * 1000)
      .toISOString()
      .replace('T', ' ')
      .slice(0, 19) + ' UTC';
  };

  /**
   * Format bytes to human-readable size
   * @param {number} bytes - Size in bytes
   * @returns {string} Formatted size
   */
  Ferrous.formatBytes = function(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return (bytes / Math.pow(k, i)).toFixed(2) + ' ' + sizes[i];
  };

  /**
   * Format hashrate with appropriate unit
   * @param {number} hashrate - Hashrate in H/s
   * @returns {string} Formatted hashrate
   */
  Ferrous.formatHashrate = function(hashrate) {
    if (hashrate >= 1e12) return (hashrate / 1e12).toFixed(2) + ' TH/s';
    if (hashrate >= 1e9) return (hashrate / 1e9).toFixed(2) + ' GH/s';
    if (hashrate >= 1e6) return (hashrate / 1e6).toFixed(2) + ' MH/s';
    if (hashrate >= 1e3) return (hashrate / 1e3).toFixed(2) + ' KH/s';
    return hashrate.toFixed(2) + ' H/s';
  };

  // ═══════════════════════════════════════════════════════════
  // RANDOM DATA GENERATORS
  // ═══════════════════════════════════════════════════════════

  /**
   * Generate a random integer between min and max (inclusive)
   * @param {number} min - Minimum value
   * @param {number} max - Maximum value
   * @returns {number} Random integer
   */
  Ferrous.randomInt = function(min, max) {
    return Math.floor(Math.random() * (max - min + 1)) + min;
  };

  /**
   * Generate a random float between min and max
   * @param {number} min - Minimum value
   * @param {number} max - Maximum value
   * @returns {number} Random float
   */
  Ferrous.randomFloat = function(min, max) {
    return Math.random() * (max - min) + min;
  };

  /**
   * Generate a random Ferrous address
   * @returns {string} Random address
   */
  Ferrous.randomAddress = function() {
    const chars = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
    let addr = '1F';
    for (let i = 0; i < 30; i++) {
      addr += chars[Math.floor(Math.random() * chars.length)];
    }
    return addr;
  };

  /**
   * Generate a random transaction ID
   * @returns {string} Random 64-character hex string
   */
  Ferrous.randomTxId = function() {
    return Ferrous.randomHex(64);
  };

  // ═══════════════════════════════════════════════════════════
  // CLIPBOARD UTILITIES
  // ═══════════════════════════════════════════════════════════

  /**
   * Copy text to clipboard with visual feedback
   * @param {string} text - Text to copy
   * @param {HTMLElement} element - Element to show feedback on (optional)
   * @returns {Promise<boolean>} Success status
   */
  Ferrous.copyToClipboard = async function(text, element) {
    try {
      await navigator.clipboard.writeText(text);
      Ferrous.showToast('Copied to clipboard', 'success');
      
      if (element) {
        const originalText = element.textContent;
        element.textContent = 'Copied!';
        element.style.color = 'var(--color-success)';
        setTimeout(function() {
          element.textContent = originalText;
          element.style.color = '';
        }, 1500);
      }
      return true;
    } catch (err) {
      Ferrous.showToast('Failed to copy', 'error');
      return false;
    }
  };

  // ═══════════════════════════════════════════════════════════
  // TOAST NOTIFICATIONS
  // ═══════════════════════════════════════════════════════════

  let toastContainer = null;

  /**
   * Show a toast notification
   * @param {string} message - Message to display
   * @param {string} type - Toast type: 'success', 'error', 'warning', 'info'
   * @param {number} duration - Duration in ms (default 3000)
   */
  Ferrous.showToast = function(message, type, duration) {
    type = type || 'info';
    duration = duration || 3000;

    // Create container if not exists
    if (!toastContainer) {
      toastContainer = document.createElement('div');
      toastContainer.className = 'toast-container';
      toastContainer.setAttribute('role', 'alert');
      toastContainer.setAttribute('aria-live', 'polite');
      document.body.appendChild(toastContainer);
    }

    // Create toast
    const toast = document.createElement('div');
    toast.className = 'toast ' + type;
    toast.innerHTML = 
      '<span>' + Ferrous.escapeHtml(message) + '</span>' +
      '<button class="toast-close" aria-label="Close notification">×</button>';

    // Add close handler
    const closeBtn = toast.querySelector('.toast-close');
    closeBtn.addEventListener('click', function() {
      removeToast(toast);
    });

    toastContainer.appendChild(toast);

    // Auto-remove
    setTimeout(function() {
      removeToast(toast);
    }, duration);
  };

  function removeToast(toast) {
    toast.style.opacity = '0';
    toast.style.transform = 'translateX(100%)';
    setTimeout(function() {
      if (toast.parentNode) {
        toast.parentNode.removeChild(toast);
      }
    }, 300);
  }

  // ═══════════════════════════════════════════════════════════
  // THEME MANAGEMENT
  // ═══════════════════════════════════════════════════════════

  /**
   * Get current theme
   * @returns {string} 'dark' or 'light'
   */
  Ferrous.getTheme = function() {
    return localStorage.getItem('ferrous-theme') || 'dark';
  };

  /**
   * Set theme
   * @param {string} theme - 'dark' or 'light'
   */
  Ferrous.setTheme = function(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem('ferrous-theme', theme);
    
    // Update theme toggle button text if exists
    const toggleBtn = document.querySelector('.theme-toggle');
    if (toggleBtn) {
      toggleBtn.textContent = theme === 'dark' ? '[ LIGHT ]' : '[ DARK ]';
      toggleBtn.setAttribute('aria-label', 'Switch to ' + (theme === 'dark' ? 'light' : 'dark') + ' theme');
    }
  };

  /**
   * Toggle between dark and light themes
   */
  Ferrous.toggleTheme = function() {
    const current = Ferrous.getTheme();
    Ferrous.setTheme(current === 'dark' ? 'light' : 'dark');
  };

  /**
   * Initialize theme from localStorage
   */
  Ferrous.initTheme = function() {
    const savedTheme = Ferrous.getTheme();
    Ferrous.setTheme(savedTheme);
  };

  // ═══════════════════════════════════════════════════════════
  // MOBILE NAVIGATION
  // ═══════════════════════════════════════════════════════════

  /**
   * Initialize mobile navigation toggle
   */
  Ferrous.initMobileNav = function() {
    const toggle = document.querySelector('.nav-toggle');
    const navLinks = document.querySelector('.nav-links');
    
    if (toggle && navLinks) {
      toggle.addEventListener('click', function() {
        const isOpen = navLinks.classList.toggle('open');
        toggle.setAttribute('aria-expanded', isOpen);
        toggle.textContent = isOpen ? '[ CLOSE ]' : '[ MENU ]';
      });

      // Close menu when clicking outside
      document.addEventListener('click', function(e) {
        if (!toggle.contains(e.target) && !navLinks.contains(e.target)) {
          navLinks.classList.remove('open');
          toggle.setAttribute('aria-expanded', 'false');
          toggle.textContent = '[ MENU ]';
        }
      });

      // Close menu on escape key
      document.addEventListener('keydown', function(e) {
        if (e.key === 'Escape' && navLinks.classList.contains('open')) {
          navLinks.classList.remove('open');
          toggle.setAttribute('aria-expanded', 'false');
          toggle.textContent = '[ MENU ]';
          toggle.focus();
        }
      });
    }
  };

  // ═══════════════════════════════════════════════════════════
  // PERFORMANCE UTILITIES
  // ═══════════════════════════════════════════════════════════

  /**
   * Debounce a function
   * @param {Function} func - Function to debounce
   * @param {number} wait - Wait time in ms
   * @returns {Function} Debounced function
   */
  Ferrous.debounce = function(func, wait) {
    let timeout;
    return function() {
      const context = this;
      const args = arguments;
      clearTimeout(timeout);
      timeout = setTimeout(function() {
        func.apply(context, args);
      }, wait);
    };
  };

  /**
   * Throttle a function
   * @param {Function} func - Function to throttle
   * @param {number} limit - Limit in ms
   * @returns {Function} Throttled function
   */
  Ferrous.throttle = function(func, limit) {
    let inThrottle;
    return function() {
      const context = this;
      const args = arguments;
      if (!inThrottle) {
        func.apply(context, args);
        inThrottle = true;
        setTimeout(function() {
          inThrottle = false;
        }, limit);
      }
    };
  };

  /**
   * Request animation frame wrapper with throttling
   * @param {Function} callback - Animation callback
   * @returns {Function} Cancel function
   */
  Ferrous.animationLoop = function(callback) {
    let running = true;
    let lastTime = 0;
    const fps = 60;
    const interval = 1000 / fps;

    function loop(currentTime) {
      if (!running) return;
      
      requestAnimationFrame(loop);
      
      const delta = currentTime - lastTime;
      if (delta >= interval) {
        lastTime = currentTime - (delta % interval);
        callback(currentTime, delta);
      }
    }

    requestAnimationFrame(loop);

    return function() {
      running = false;
    };
  };

  // ═══════════════════════════════════════════════════════════
  // DOM UTILITIES
  // ═══════════════════════════════════════════════════════════

  /**
   * Escape HTML special characters
   * @param {string} text - Text to escape
   * @returns {string} Escaped text
   */
  Ferrous.escapeHtml = function(text) {
    const div = document.createElement('div');
    div.textContent = text;
    return div.innerHTML;
  };

  /**
   * Create element with attributes
   * @param {string} tag - HTML tag
   * @param {Object} attrs - Attributes
   * @param {string|HTMLElement|Array} content - Content
   * @returns {HTMLElement} Created element
   */
  Ferrous.createElement = function(tag, attrs, content) {
    const el = document.createElement(tag);
    
    if (attrs) {
      Object.keys(attrs).forEach(function(key) {
        if (key === 'class') {
          el.className = attrs[key];
        } else if (key === 'style' && typeof attrs[key] === 'object') {
          Object.assign(el.style, attrs[key]);
        } else if (key.startsWith('data-')) {
          el.setAttribute(key, attrs[key]);
        } else if (key.startsWith('on') && typeof attrs[key] === 'function') {
          el.addEventListener(key.slice(2).toLowerCase(), attrs[key]);
        } else {
          el[key] = attrs[key];
        }
      });
    }

    if (content) {
      if (typeof content === 'string') {
        el.innerHTML = content;
      } else if (content instanceof HTMLElement) {
        el.appendChild(content);
      } else if (Array.isArray(content)) {
        content.forEach(function(child) {
          if (typeof child === 'string') {
            el.appendChild(document.createTextNode(child));
          } else if (child instanceof HTMLElement) {
            el.appendChild(child);
          }
        });
      }
    }

    return el;
  };

  // ═══════════════════════════════════════════════════════════
  // KEYBOARD SHORTCUTS
  // ═══════════════════════════════════════════════════════════

  const shortcuts = {};

  /**
   * Register a keyboard shortcut
   * @param {string} key - Key combination (e.g., 'ctrl+k', 'escape')
   * @param {Function} callback - Callback function
   * @param {string} description - Description for help menu
   */
  Ferrous.registerShortcut = function(key, callback, description) {
    shortcuts[key.toLowerCase()] = { callback: callback, description: description };
  };

  /**
   * Initialize keyboard shortcuts listener
   */
  Ferrous.initShortcuts = function() {
    document.addEventListener('keydown', function(e) {
      // Don't trigger shortcuts when typing in inputs
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') {
        return;
      }

      let key = '';
      if (e.ctrlKey || e.metaKey) key += 'ctrl+';
      if (e.altKey) key += 'alt+';
      if (e.shiftKey) key += 'shift+';
      key += e.key.toLowerCase();

      if (shortcuts[key]) {
        e.preventDefault();
        shortcuts[key].callback(e);
      }
    });
  };

  // ═══════════════════════════════════════════════════════════
  // CSV EXPORT
  // ═══════════════════════════════════════════════════════════

  /**
   * Export data as CSV file
   * @param {Array} data - Array of objects
   * @param {string} filename - Filename for download
   * @param {Array} columns - Column definitions [{key, label}]
   */
  Ferrous.exportCSV = function(data, filename, columns) {
    if (!data || !data.length) {
      Ferrous.showToast('No data to export', 'warning');
      return;
    }

    // Build header row
    const headers = columns.map(function(col) {
      return '"' + (col.label || col.key).replace(/"/g, '""') + '"';
    });

    // Build data rows
    const rows = data.map(function(item) {
      return columns.map(function(col) {
        let value = item[col.key];
        if (value === null || value === undefined) value = '';
        value = String(value).replace(/"/g, '""');
        return '"' + value + '"';
      }).join(',');
    });

    const csv = [headers.join(',')].concat(rows).join('\n');
    const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
    const link = document.createElement('a');
    
    if (navigator.msSaveBlob) {
      navigator.msSaveBlob(blob, filename);
    } else {
      link.href = URL.createObjectURL(blob);
      link.download = filename;
      link.style.display = 'none';
      document.body.appendChild(link);
      link.click();
      document.body.removeChild(link);
    }

    Ferrous.showToast('Exported ' + data.length + ' rows', 'success');
  };

  // ═══════════════════════════════════════════════════════════
  // INITIALIZATION
  // ═══════════════════════════════════════════════════════════

  /**
   * Initialize all Ferrous utilities
   */
  Ferrous.init = function() {
    // Initialize theme
    Ferrous.initTheme();

    // Initialize mobile navigation
    Ferrous.initMobileNav();

    // Initialize keyboard shortcuts
    Ferrous.initShortcuts();

    // Register default shortcuts
    Ferrous.registerShortcut('t', Ferrous.toggleTheme, 'Toggle theme');
    Ferrous.registerShortcut('escape', function() {
      // Close any open modals or panels
      const openPanels = document.querySelectorAll('.visible, .open');
      openPanels.forEach(function(panel) {
        panel.classList.remove('visible', 'open');
      });
    }, 'Close panels');

    // Add theme toggle handler
    const themeToggle = document.querySelector('.theme-toggle');
    if (themeToggle) {
      themeToggle.addEventListener('click', Ferrous.toggleTheme);
    }
  };

  // Auto-initialize when DOM is ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', Ferrous.init);
  } else {
    Ferrous.init();
  }

  // ═══════════════════════════════════════════════════════════
  // EXPORT
  // ═══════════════════════════════════════════════════════════
  global.Ferrous = Ferrous;

})(typeof window !== 'undefined' ? window : this);
