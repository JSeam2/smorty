# Smorty Production Deployment

This directory contains production deployment configurations for the Smorty indexer application.

## Architecture

The production setup consists of three services:

1. **PostgreSQL 18** - Database for storing indexed blockchain data
2. **Smorty App** - Rust-based blockchain indexer with dynamic API endpoints
3. **Nginx** - Reverse proxy with SSL/TLS termination

## Prerequisites

- Docker Engine 20.10+
- Docker Compose 2.0+
- SSL certificates (see SSL Certificate Setup below)
- `config.toml` file in project root
- ABI files in `abi/` directory

## Quick Start

### 1. Setup Environment

```bash
cd production
cp .env.example .env
# Edit .env with your configuration
nano .env
```

### 2. Setup SSL Certificates

Place your SSL certificates in the `nginx/ssl/` directory:

1. `cert.pem` - Your SSL certificate (or certificate chain)
2. `key.pem` - Your private key

#### Certificate Options

**Option 1: Commercial SSL Certificate (e.g., Let's Encrypt, DigiCert, etc.)**

If using Let's Encrypt with certbot:
```bash
# Get certificates
certbot certonly --standalone -d yourdomain.com

# Copy to production directory
cd production
cp /etc/letsencrypt/live/yourdomain.com/fullchain.pem nginx/ssl/cert.pem
cp /etc/letsencrypt/live/yourdomain.com/privkey.pem nginx/ssl/key.pem
```

**Option 2: Cloudflare Origin Certificates**

1. Log in to your Cloudflare dashboard
2. Select your domain
3. Go to SSL/TLS > Origin Server
4. Click "Create Certificate"
5. Copy the origin certificate to `nginx/ssl/cert.pem`
6. Copy the private key to `nginx/ssl/key.pem`

**Option 3: Self-Signed Certificate (Development/Testing Only)**

```bash
cd production/nginx/ssl
openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem \
  -subj "/C=US/ST=State/L=City/O=Organization/CN=yourdomain.com"
```

**Set File Permissions:**
```bash
cd production
chmod 600 nginx/ssl/key.pem
chmod 644 nginx/ssl/cert.pem
```

**Important Security Notes:**
- Never commit these files to version control
- Add `*.pem` and `*.key` to your `.gitignore`
- Keep your private key secure and backed up safely
- Rotate certificates before expiration

**Certificate Renewal:**
- Let's Encrypt certificates expire every 90 days
- Commercial certificates typically last 1 year
- Cloudflare origin certificates can last up to 15 years
- Set up automated renewal for production environments

### 3. Configure Application

Ensure `config.toml` exists in the project root:

```bash
cd ..
cp config.toml.example config.toml
# Edit config.toml with your settings
nano config.toml
```

### 4. Update Nginx Server Name

Edit `nginx/conf.d/smorty.conf` and replace `server_name _;` with your domain:

```nginx
server_name yourdomain.com;
```

### 5. Deploy

```bash
cd production
docker-compose -f docker-compose-prod.yaml up -d
```

## Management Commands

### View Logs

```bash
# All services
docker-compose -f docker-compose-prod.yaml logs -f

# Specific service
docker-compose -f docker-compose-prod.yaml logs -f smorty-app
docker-compose -f docker-compose-prod.yaml logs -f postgres
docker-compose -f docker-compose-prod.yaml logs -f nginx
```

### Stop Services

```bash
docker-compose -f docker-compose-prod.yaml down
```

### Restart Services

```bash
docker-compose -f docker-compose-prod.yaml restart
```

### Rebuild Application

```bash
docker-compose -f docker-compose-prod.yaml up -d --build smorty-app
```

### Database Backup

```bash
# Backup
docker exec smorty-postgres pg_dump -U postgres smorty > backup_$(date +%Y%m%d_%H%M%S).sql

# Restore
cat backup.sql | docker exec -i smorty-postgres psql -U postgres -d smorty
```

## Kubernetes Deployment

The Dockerfile is also optimized for Kubernetes deployment. Here's a basic example:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: smorty-app
spec:
  replicas: 3
  selector:
    matchLabels:
      app: smorty
  template:
    metadata:
      labels:
        app: smorty
    spec:
      containers:
      - name: smorty
        image: your-registry/smorty:latest
        ports:
        - containerPort: 8080
        env:
        - name: DATABASE_URI
          valueFrom:
            secretKeyRef:
              name: smorty-secrets
              key: database-uri
        - name: RUST_LOG
          value: "info"
        volumeMounts:
        - name: config
          mountPath: /app/config
          readOnly: true
        - name: abi
          mountPath: /app/abi
          readOnly: true
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 30
          periodSeconds: 10
        readinessProbe:
          httpGet:
            path: /health
            port: 8080
          initialDelaySeconds: 5
          periodSeconds: 5
      volumes:
      - name: config
        configMap:
          name: smorty-config
      - name: abi
        configMap:
          name: smorty-abi
```

## Health Checks

- Application: `https://yourdomain.com/health`
- Swagger UI: `https://yourdomain.com/swagger-ui/`

## Security Considerations

1. **Secrets Management**
   - Never commit `.env` file
   - Use strong database passwords
   - Rotate credentials regularly

2. **SSL/TLS**
   - Keep certificates up to date
   - Use strong cipher suites (configured in nginx.conf)
   - Consider enabling HSTS after testing

3. **Network Security**
   - Database is not exposed to host (only internal network)
   - Application is not exposed to host (only through nginx)
   - Rate limiting is enabled (10 req/s with burst of 20)

4. **Container Security**
   - Application runs as non-root user (uid 1000)
   - Minimal base image (debian:bookworm-slim)
   - Only necessary runtime dependencies installed

## Monitoring

### Container Health

```bash
docker-compose -f docker-compose-prod.yaml ps
```

### Resource Usage

```bash
docker stats smorty-postgres smorty-app smorty-nginx
```

### Nginx Access Logs

```bash
tail -f nginx/logs/access.log
```

### Nginx Error Logs

```bash
tail -f nginx/logs/error.log
```

## Troubleshooting

### Application won't start

```bash
# Check logs
docker-compose -f docker-compose-prod.yaml logs smorty-app

# Check database connectivity
docker exec smorty-app ping postgres
```

### Database connection issues

```bash
# Check postgres is healthy
docker-compose -f docker-compose-prod.yaml ps postgres

# Check database logs
docker-compose -f docker-compose-prod.yaml logs postgres

# Test connection
docker exec -it smorty-postgres psql -U postgres -d smorty
```

### SSL certificate issues

```bash
# Verify certificate
openssl x509 -in nginx/ssl/cert.pem -text -noout

# Test SSL connection
openssl s_client -connect yourdomain.com:443
```

## Performance Tuning

### PostgreSQL

Edit `docker-compose-prod.yaml` to add PostgreSQL performance settings:

```yaml
postgres:
  command:
    - "postgres"
    - "-c"
    - "max_connections=200"
    - "-c"
    - "shared_buffers=256MB"
    - "-c"
    - "effective_cache_size=1GB"
```

### Nginx

Adjust worker processes in `nginx/nginx.conf` based on your CPU cores.

### Application Scaling

For horizontal scaling:

```bash
docker-compose -f docker-compose-prod.yaml up -d --scale smorty-app=3
```

Note: You'll need to configure nginx upstream load balancing for multiple app instances.

## Maintenance

### Update Application

```bash
cd production
docker-compose -f docker-compose-prod.yaml pull
docker-compose -f docker-compose-prod.yaml up -d --build
```

### Clean Up

```bash
# Remove unused images
docker image prune -a

# Remove unused volumes (careful!)
docker volume prune
```

## Support

For issues or questions, please refer to the main project repository.
